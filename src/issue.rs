//! GitHub issue triggers: parsing a `#123` or issue-URL reference out of a
//! tag, and resolving each one to open or closed.
//!
//! This is the only part of `todo-by` that touches the network, and it runs
//! only when `--online` (or `online = true`) is set AND the scan actually
//! found an issue tag. Everything else in the tool stays hermetic.
//!
//! Firing rule: any state other than `OPEN` fires, `MERGED` included. The
//! close reason (`completed`, `not_planned`) is never requested and never
//! inspected. A tool that weighs why a tracker was closed is reading intent
//! out of an enum; the author reads the finding and decides.
//!
//! There is no HTTP client here. Both transports are subprocesses, which
//! keeps the dependency list at one crate: `curl` when a token is in the
//! environment (it is the transport with a real timeout, and CI is where a
//! hung job hurts), otherwise `gh` (which authenticates from its own
//! keyring, so a laptop needs no setup at all). Both speak GraphQL and
//! share one request body, one response reader, and one error mapper.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::json::{self, Json};

/// Seconds before the curl transport gives up. `gh` has no equivalent flag,
/// so the `gh` path relies on the user noticing an interactive hang, the
/// same way `version-cmd` and `git describe` already do.
const CURL_TIMEOUT: &str = "30";

/// References per request. GitHub's GraphQL API imposes no alias cap, but a
/// bounded batch keeps one unlucky repository from producing a
/// multi-megabyte query, and 100 covers nearly every repository in one trip.
const BATCH: usize = 100;

const REMEDY: &str = "authenticate with `gh auth login`, or set GH_TOKEN \
                      (in GitHub Actions: permissions: issues: read)";

/// A repository an issue reference points at. `host` is a bare hostname
/// (`github.com`, or a GitHub Enterprise host, optionally with a port), so
/// the API base is derived rather than stored.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Repo {
    pub host: String,
    pub owner: String,
    pub name: String,
}

impl Repo {
    /// Parses `owner/name` (the `repo` config key). The charsets are
    /// GitHub's own, and validating them here is also what makes it safe to
    /// interpolate these strings into a GraphQL query below.
    pub fn parse_slug(slug: &str, host: &str) -> Option<Repo> {
        let (owner, name) = slug.split_once('/')?;
        if !valid_owner(owner) || !valid_name(name) {
            return None;
        }
        Some(Repo {
            host: host.to_string(),
            owner: owner.to_string(),
            name: name.to_string(),
        })
    }

    fn graphql_url(&self) -> String {
        if self.host == "github.com" {
            "https://api.github.com/graphql".to_string()
        } else {
            format!("https://{}/api/graphql", self.host)
        }
    }

    fn issue_url(&self, number: u32) -> String {
        format!(
            "https://{}/{}/{}/issues/{number}",
            self.host, self.owner, self.name
        )
    }
}

fn valid_owner(s: &str) -> bool {
    !s.is_empty() && s.len() <= 39 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Canonical form of a hostname, applied to every host this tool takes in,
/// whether from a git remote or from a URL in a comment.
///
/// Hostnames are case-insensitive, and the github.com test in
/// `graphql_url` (and the one confining the environment token) is an
/// equality check, so `GitHub.com` and `github.com:443` must not read as
/// different hosts. `ssh.github.com` is how a clone escapes a firewall that
/// blocks port 22; it is github.com.
fn normalize_host(host: &str) -> String {
    let host = host.to_ascii_lowercase();
    let host = host
        .strip_suffix(":443")
        .or_else(|| host.strip_suffix(":80"))
        .unwrap_or(&host);
    if host == "ssh.github.com" {
        "github.com".to_string()
    } else {
        host.to_string()
    }
}

fn valid_host(s: &str) -> bool {
    !s.is_empty()
        && !s.contains('@')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'))
}

/// A reference written in a tag. `repo: None` is the `#123` form, whose
/// repository comes from the git remote (or the `repo` config key) and is
/// resolved once per run rather than once per tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub repo: Option<Repo>,
    pub number: u32,
}

impl Reference {
    /// Parses the two spellings: `#123`, and a full issue or pull-request
    /// URL. `owner/repo#123` is deliberately absent: it says nothing the
    /// URL cannot, and it cannot name a host, so a GitHub Enterprise
    /// reference would need the URL anyway. One spelling per job.
    pub fn parse(written: &str) -> Option<Reference> {
        if let Some(digits) = written.strip_prefix('#') {
            return Some(Reference {
                repo: None,
                number: parse_number(digits)?,
            });
        }
        let rest = written
            .strip_prefix("https://")
            .or_else(|| written.strip_prefix("http://"))?;
        // A comment permalink (`.../issues/123#issuecomment-456`) and a
        // tracking query are both ordinary things to paste, and neither
        // changes which issue is meant. Without this the number segment
        // fails to parse and the whole tag is downgraded to prose: an
        // unambiguously intended trigger that never fires and never says so.
        let rest = rest.split(['#', '?']).next()?;
        let (host, path) = rest.split_once('/')?;
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let [owner, name, kind, number] = segments[..] else {
            return None;
        };
        if !matches!(kind, "issues" | "pull") || !valid_host(host) {
            return None;
        }
        Some(Reference {
            repo: Some(Repo::parse_slug(
                &format!("{owner}/{name}"),
                &normalize_host(host),
            )?),
            number: parse_number(number)?,
        })
    }
}

/// Issue numbers are bounded well below `u32::MAX` in practice; the cap
/// exists so a pasted timestamp is rejected as a typo rather than sent to
/// the API as a lookup that can only 404.
fn parse_number(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match s.parse::<u32>() {
        Ok(n) if n > 0 && n <= 9_999_999 => Some(n),
        _ => None,
    }
}

/// Why a `#`-shaped span isn't a usable reference. Only the `#` form
/// reaches this: `#` is a marker, so a tag carrying one meant a reference
/// and earns an error naming the spelling. A URL is not a marker, so the
/// scanner leaves an unrecognized one as prose instead of reporting it.
pub fn syntax_help(written: &str) -> String {
    if written.starts_with('#') {
        format!("invalid issue reference {written:?} (write #123, or the full issue URL)")
    } else {
        format!("invalid issue URL {written:?} (write https://github.com/owner/repo/issues/123)")
    }
}

/// What a resolved reference turned out to be.
pub enum Outcome {
    /// Still open: no finding.
    Open,
    /// Closed or merged, so the tag fires. `state` is the word to print.
    Fired { state: &'static str, url: String },
    /// This one reference could not be resolved (unknown repository, no
    /// access, no git remote). Others in the same run are unaffected.
    Failed(String),
    /// The request carrying this reference failed outright, and that failure
    /// was already reported once for the whole request. Dropped silently
    /// here so one dead host doesn't print the same sentence per tag.
    Unanswered,
}

/// Resolves every reference in `refs`, in order, returning one outcome per
/// reference.
///
/// The second half of the return is a failure that made a whole request
/// meaningless (no transport, a rejected token, an unreachable host). It is
/// reported once rather than once per tag, and it abandons only the host it
/// happened on; every answer already collected is kept. A rate limit on the
/// second batch says nothing about the closed issue the first one
/// confirmed, and the run exits 2 anyway, so keeping that finding cannot be
/// read as "everything else is fine". References nobody got an answer for
/// are `Unanswered` and simply disappear. Only the first failure comes
/// back, since one sentence per run is the point.
pub fn resolve(
    refs: &[Reference],
    configured: Option<&Repo>,
    dir: &Path,
) -> (Vec<Outcome>, Option<String>) {
    if refs.is_empty() {
        return (Vec::new(), None);
    }

    // The git remote is consulted at most once per run, and only if some
    // tag actually used the bare `#123` form.
    let mut default: Option<Result<Repo, String>> = configured.cloned().map(Ok);
    let mut targets: Vec<Result<(usize, u32), String>> = Vec::with_capacity(refs.len());
    let mut repos: Vec<Repo> = Vec::new();
    for r in refs {
        let repo = match &r.repo {
            Some(repo) => Ok(repo.clone()),
            None => default.get_or_insert_with(|| default_repo(dir)).clone(),
        };
        targets.push(repo.map(|repo| {
            let idx = repos.iter().position(|r| *r == repo).unwrap_or_else(|| {
                repos.push(repo);
                repos.len() - 1
            });
            (idx, r.number)
        }));
    }

    // One transport per host, not per request: `have` spawns a process to
    // decide, and a repository with several batches would otherwise pay that
    // cost on every one of them.
    let mut transport: Option<(String, Transport)> = None;
    let mut states: HashMap<(usize, u32), Result<String, String>> = HashMap::new();
    let mut failure = None;
    // A host that fails is abandoned, and only that host: an unreachable
    // enterprise instance says nothing about the github.com references in
    // the same tree, and letting it decide their fate would make the
    // reported findings depend on which host happened to be tried first.
    let mut dead: Vec<String> = Vec::new();
    for batch in batches(&targets, &repos) {
        let repo = &repos[batch[0].0];
        if dead.contains(&repo.host) {
            continue;
        }
        if transport
            .as_ref()
            .is_none_or(|(host, _)| *host != repo.host)
        {
            match choose_transport(&repo.host, env_token()) {
                Ok(picked) => transport = Some((repo.host.clone(), picked)),
                Err(err) => {
                    failure.get_or_insert(err);
                    dead.push(repo.host.clone());
                    continue;
                }
            }
        }
        let (url, body) = request(&repos, &batch);
        let sent = transport
            .as_ref()
            .expect("transport chosen above")
            .1
            .send(repo, &url, &body)
            .and_then(|response| read_response(&response, &batch, &mut states));
        if let Err(err) = sent {
            failure.get_or_insert(err);
            dead.push(repo.host.clone());
        }
    }

    let outcomes = targets
        .into_iter()
        .map(|target| {
            let key = match target {
                Ok(key) => key,
                Err(err) => return Outcome::Failed(err),
            };
            match states.get(&key) {
                Some(Ok(state)) if state == "OPEN" => Outcome::Open,
                Some(Ok(state)) => Outcome::Fired {
                    state: if state == "MERGED" {
                        "merged"
                    } else {
                        "closed"
                    },
                    url: repos[key.0].issue_url(key.1),
                },
                Some(Err(err)) => Outcome::Failed(err.clone()),
                // Unreachable while `failure` is None: every reference in
                // `targets` was in some batch, and a batch that returned
                // filled all of its own keys.
                None => Outcome::Unanswered,
            }
        })
        .collect();
    (outcomes, failure)
}

/// Splits the resolved targets into per-host batches of at most [`BATCH`]
/// distinct references. Grouping by host is what keeps a run that mixes
/// github.com with a GitHub Enterprise host correct: one request can only
/// ever reach one API.
fn batches(targets: &[Result<(usize, u32), String>], repos: &[Repo]) -> Vec<Vec<(usize, u32)>> {
    let mut unique = std::collections::HashSet::new();
    let seen: Vec<(usize, u32)> = targets
        .iter()
        .flatten()
        .copied()
        .filter(|key| unique.insert(*key))
        .collect();
    let mut out: Vec<Vec<(usize, u32)>> = Vec::new();
    for host in hosts(repos) {
        let mut keys: Vec<(usize, u32)> = seen
            .iter()
            .copied()
            .filter(|(repo, _)| repos[*repo].host == host)
            .collect();
        // Sorting by repository is load-bearing, not tidiness: `request`
        // opens a new `rN:` block every time the repository changes, so
        // references interleaved across repositories in source order
        // (`r0#1`, `r1#2`, `r0#3`) would emit the `r0` alias twice in one
        // query and the API would reject the whole batch. The sort is
        // stable, so numbers keep their source order inside each block.
        keys.sort_by_key(|(repo, _)| *repo);
        out.extend(keys.chunks(BATCH).map(<[(usize, u32)]>::to_vec));
    }
    out
}

fn hosts(repos: &[Repo]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for repo in repos {
        if !out.contains(&repo.host) {
            out.push(repo.host.clone());
        }
    }
    out
}

/// Builds the GraphQL request for one batch: the endpoint URL, and the JSON
/// body both transports post verbatim.
///
/// `issueOrPullRequest` is what lets the syntax stay one form: `#123` needs
/// no advance knowledge of whether 123 is an issue or a PR. Only `state` is
/// selected, since the close reason is never inspected.
///
/// Interpolating `owner` and `name` straight into the query is safe because
/// [`valid_owner`] and [`valid_name`] already rejected everything outside
/// GitHub's own charsets, neither of which contains a quote or a brace.
fn request(repos: &[Repo], batch: &[(usize, u32)]) -> (String, String) {
    let mut query = String::from("query {");
    let mut current: Option<usize> = None;
    for (repo, number) in batch {
        if current != Some(*repo) {
            if current.is_some() {
                query.push_str(" }");
            }
            let Repo { owner, name, .. } = &repos[*repo];
            query.push_str(&format!(
                " r{repo}: repository(owner: \"{owner}\", name: \"{name}\") {{"
            ));
            current = Some(*repo);
        }
        query.push_str(&format!(
            " n{number}: issueOrPullRequest(number: {number}) \
             {{ ... on Issue {{ state }} ... on PullRequest {{ state }} }}"
        ));
    }
    query.push_str(" } }");
    // The query is built here, from validated parts, so the only character
    // needing escaping is the quote around each owner and name. It is also
    // deliberately a single line, which keeps that true.
    let body = format!("{{\"query\":\"{}\"}}", query.replace('"', "\\\""));
    (repos[batch[0].0].graphql_url(), body)
}

/// Reads one response into `states`. GraphQL answers partially: a reference
/// nobody can see arrives as a null alias plus an `errors[]` entry naming
/// its path, in the same round trip as the successes. That is the whole
/// reason this uses GraphQL rather than one REST call per reference.
fn read_response(
    response: &str,
    batch: &[(usize, u32)],
    states: &mut HashMap<(usize, u32), Result<String, String>>,
) -> Result<(), String> {
    let json = json::parse(response).ok_or_else(|| {
        format!(
            "could not read the GitHub API response: {}",
            excerpt(response)
        )
    })?;
    let data = json.get("data").filter(|d| **d != Json::Null);
    let errors: Vec<(Vec<&str>, String)> = json
        .get("errors")
        .and_then(Json::as_arr)
        .unwrap_or_default()
        .iter()
        .map(|e| {
            let path = e
                .get("path")
                .and_then(Json::as_arr)
                .unwrap_or_default()
                .iter()
                .filter_map(Json::as_str)
                .collect();
            let message = e
                .get("message")
                .and_then(Json::as_str)
                .map(sanitize)
                .unwrap_or_else(|| "unspecified error".to_string());
            (path, message)
        })
        .collect();
    // A response carrying errors but no data at all is a whole-batch
    // failure (a malformed query, a revoked token), not a per-reference one.
    let Some(data) = data else {
        let detail = errors
            .first()
            .map(|(_, message)| message.clone())
            .unwrap_or_else(|| excerpt(response));
        return Err(format!("the GitHub API rejected the request: {detail}"));
    };

    for (repo, number) in batch {
        let repo_alias = format!("r{repo}");
        let number_alias = format!("n{number}");
        let state = data
            .get(&repo_alias)
            .and_then(|r| r.get(&number_alias))
            .and_then(|n| n.get("state"))
            .and_then(Json::as_str);
        let entry = match state {
            Some(state) => Ok(state.to_string()),
            // The reference's own error wins; matching on the repository
            // alias alone would print the first missing issue's message for
            // every later one. The repository-level fallback is still
            // needed, since a repository that doesn't exist produces one
            // error whose path is just ["rN"] and explains all of them.
            None => Err(errors
                .iter()
                .find(|(path, _)| path.starts_with(&[repo_alias.as_str(), number_alias.as_str()]))
                .or_else(|| {
                    errors
                        .iter()
                        .find(|(path, _)| path[..] == [repo_alias.as_str()])
                })
                .map(|(_, message)| message.clone())
                .unwrap_or_else(|| "not found".to_string())),
        };
        states.insert((*repo, *number), entry);
    }
    Ok(())
}

/// Strips control characters from anything a server said before it reaches
/// a terminal. Response bodies and GraphQL messages are remote input, and
/// an escape sequence in one would otherwise repaint the user's screen from
/// inside a finding.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c == '\n' || c == '\t' || !c.is_control() {
                c
            } else {
                ' '
            }
        })
        .collect()
}

/// First non-empty line of an unreadable response, for an error message.
/// Truncated because an HTML error page from a proxy is otherwise thousands
/// of lines.
fn excerpt(response: &str) -> String {
    let cleaned = sanitize(response);
    let line = cleaned.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    if line.len() > 200 {
        // Byte-slicing a response from somebody else's proxy is how a
        // reporting path panics: the cut must land on a character boundary.
        let end = (0..=200)
            .rev()
            .find(|&i| line.is_char_boundary(i))
            .unwrap_or(0);
        format!("{}...", &line[..end])
    } else {
        line.to_string()
    }
}

enum Transport {
    /// The token travels in curl's config file on stdin, never in argv.
    Curl(String),
    Gh,
}

/// Picks a transport for one host.
///
/// A token in the environment selects curl, because curl is the transport
/// that can be told to give up (`--max-time`) and an environment token
/// usually means CI, where a hung job is expensive. Without one, `gh` is
/// the only option that can authenticate at all, and it does so with no
/// setup. Either way the identity is the same: `gh` also prefers `GH_TOKEN`
/// over its keyring, so this choice never silently switches accounts.
///
/// **The environment token is only ever sent to github.com.** Hosts come
/// out of scanned comments, so a tag reading
/// `todo-by https://evil.example/o/r/issues/1` would otherwise make a CI
/// run POST `Authorization: Bearer $GITHUB_TOKEN` at whatever host the
/// comment named: repository content deciding where a credential goes.
/// Any other host is routed through `gh`, which keeps credentials per host
/// and fails closed when it has none for that one. That is also simply
/// correct for GitHub Enterprise, where a github.com token is the wrong
/// credential anyway.
fn choose_transport(host: &str, token: Option<String>) -> Result<Transport, String> {
    match token {
        Some(token) if host == "github.com" && have("curl") => Ok(Transport::Curl(token)),
        _ if have("gh") => Ok(Transport::Gh),
        Some(_) if host != "github.com" => Err(format!(
            "--online needs gh on PATH to reach {host}: an environment token is \
             only ever sent to github.com, never to a host named in a comment"
        )),
        Some(_) => Err(format!(
            "--online needs curl or gh on PATH to reach the GitHub API; {REMEDY}"
        )),
        None => Err(format!(
            "--online found no GitHub credentials and no gh on PATH; {REMEDY}"
        )),
    }
}

/// The GitHub token from the environment, if any. Empty and
/// whitespace-only values count as unset: `GH_TOKEN: ${{ secrets.X }}` with
/// no such secret expands to the empty string, and treating that as a
/// credential would pick the curl path and then fail with a 401 instead of
/// falling through to gh.
fn env_token() -> Option<String> {
    // The emptiness check belongs inside the search, not after it: filtering
    // afterwards lets an empty GH_TOKEN win the lookup and then be discarded,
    // hiding a perfectly good GITHUB_TOKEN behind it.
    ["GH_TOKEN", "GITHUB_TOKEN"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|t| !t.trim().is_empty()))
}

/// Whether a command exists, established by running it rather than by
/// searching PATH: `--version` is cheap, and PATH lookup rules (PATHEXT on
/// Windows, shell builtins) are exactly the thing not worth reimplementing.
fn have(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

impl Transport {
    fn send(&self, repo: &Repo, url: &str, body: &str) -> Result<String, String> {
        match self {
            Transport::Gh => send_gh(repo, body),
            Transport::Curl(token) => send_curl(token, url, body),
        }
    }
}

/// `gh api graphql --input -` posts the same body curl does, so both
/// transports share the request builder and the response reader.
fn send_gh(repo: &Repo, body: &str) -> Result<String, String> {
    let mut cmd = Command::new("gh");
    cmd.args(["api", "graphql", "--input", "-"]);
    if repo.host != "github.com" {
        cmd.args(["--hostname", &repo.host]);
    }
    let ran = run(cmd, body.as_bytes())?;
    if ran.status.success() {
        return Ok(ran.stdout);
    }
    // gh exits non-zero on a 200-with-errors response but still prints the
    // GraphQL body, which carries the per-reference detail; that body is
    // handed on rather than discarded. Only a failure with no readable body
    // becomes a transport error, quoting gh's own diagnosis ("gh auth
    // login", "HTTP 401") rather than re-wording it.
    if json::parse(&ran.stdout).is_some() {
        return Ok(ran.stdout);
    }
    // gh writes its own actionable diagnosis ("run: gh auth login", "check
    // your internet connection"), so it is quoted rather than re-worded and
    // the generic remedy is not appended on top of it: telling someone to
    // authenticate when the host simply did not resolve is noise. Newlines
    // collapse so the finding stays one line.
    let detail = excerpt(&ran.stderr.replace('\n', " "));
    Err(format!(
        "gh could not reach {}: {}",
        repo.host,
        if detail.is_empty() {
            ran.status.to_string()
        } else {
            detail
        }
    ))
}

/// Runs curl with the URL, headers, and body reference in a config file fed
/// on stdin. The token therefore never appears in argv (visible to every
/// process on the machine via `ps`) and never lands on disk.
///
/// The body goes in a temporary file rather than in the config block
/// because it can run to tens of kilobytes on one line and old curl builds
/// cap config line length. The file holds no secret, so writing it is safe.
fn send_curl(token: &str, url: &str, body: &str) -> Result<String, String> {
    // curl's config format is line-oriented, so a newline inside the token
    // would end the header line and let the rest be read as further
    // directives. Escaping quotes and backslashes does not cover that, and a
    // real token never contains one, so it is refused rather than sanitized.
    if token.contains(['\n', '\r']) {
        return Err(
            "the GitHub token contains a line break; check the environment variable".to_string(),
        );
    }
    let path = stage_body(body)?;
    let mut cmd = Command::new("curl");
    // No `--retry`: the response goes to stdout, which curl cannot truncate
    // the way it truncates an `-o` file, so a retried 502 would concatenate
    // two bodies and two status lines and the reader would call a request
    // that eventually succeeded unreadable.
    cmd.args([
        "--config",
        "-",
        "--no-progress-meter",
        "--max-time",
        CURL_TIMEOUT,
    ]);
    // curl unescapes `\` and `"` inside a quoted config value, so both are
    // escaped here. The token is the only value that can realistically
    // contain either.
    let config = format!(
        "url = \"{url}\"\n\
         header = \"Authorization: Bearer {}\"\n\
         header = \"Content-Type: application/json\"\n\
         data-binary = \"@{}\"\n\
         write-out = \"\\n%{{http_code}}\"\n",
        escape_curl(token),
        escape_curl(&path.display().to_string()),
    );
    let result = run(cmd, config.as_bytes());
    let _ = std::fs::remove_file(&path);
    let ran = result?;

    if !ran.status.success() {
        let code = ran.status.code().unwrap_or(-1);
        // curl's documented exit codes, mapped to the three failures worth
        // telling apart. Anything else quotes curl's own stderr.
        let detail = match code {
            6 => "could not resolve the host".to_string(),
            7 => "could not connect".to_string(),
            28 => format!("timed out after {CURL_TIMEOUT}s"),
            _ => ran.stderr.trim().to_string(),
        };
        return Err(format!("curl could not reach the GitHub API ({detail})"));
    }

    // `write-out` appended the status on its own line, so the body is
    // everything before it. `--fail` is deliberately not used: it would
    // discard the body, and GitHub puts the reason for a 401 or 403 in it.
    let (body, status) = ran
        .stdout
        .rsplit_once('\n')
        .unwrap_or((ran.stdout.as_str(), ""));
    match status.trim() {
        "200" => Ok(body.to_string()),
        "401" => Err(format!(
            "the GitHub token was rejected (HTTP 401); {REMEDY}"
        )),
        status @ ("403" | "429") => Err(format!(
            "the GitHub API refused the request (HTTP {status}): rate limit, \
             or an org requiring SSO authorization for this token"
        )),
        other => Err(format!(
            "the GitHub API returned HTTP {other}: {}",
            excerpt(body)
        )),
    }
}

fn escape_curl(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Writes the request body to a fresh file in the temp directory and
/// returns its path.
///
/// `create_new` (O_EXCL) rather than a plain write: the temp directory is
/// world-writable, so a plain write would follow a symlink somebody else
/// planted at the predictable name and truncate whatever it pointed at. It
/// also settles a collision between two `todo-by` processes that share a
/// pid (separate containers on one host), which is why the name carries a
/// counter and the loop retries instead of overwriting.
fn stage_body(body: &str) -> Result<PathBuf, String> {
    use std::io::Write;
    let fail = |err: std::io::Error| format!("could not stage the GitHub API request: {err}");
    let dir = std::env::temp_dir();
    for attempt in 0..64 {
        let path = dir.join(format!("todo-by-{}-{attempt}.json", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                return match file.write_all(body.as_bytes()) {
                    Ok(()) => Ok(path),
                    // Leaving a partial file behind would burn one of the 64
                    // names for the rest of this pid's life.
                    Err(err) => {
                        let _ = std::fs::remove_file(&path);
                        Err(fail(err))
                    }
                };
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(fail(err)),
        }
    }
    Err("could not stage the GitHub API request: no free temp file".to_string())
}

/// A finished subprocess, decoded and size-capped.
struct Ran {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

/// Largest response this will read. A GraphQL answer for 100 references is
/// a few kilobytes; the cap exists so a broken proxy streaming an endless
/// error page can't be buffered into memory (twice: once as bytes, again as
/// a parsed tree). Passing it kills the child rather than reading on.
const MAX_RESPONSE: usize = 8 * 1024 * 1024;
const MAX_STDERR: usize = 64 * 1024;

/// Reads at most `cap` bytes, reporting whether the source had more.
fn read_capped(source: impl std::io::Read, cap: usize) -> std::io::Result<(Vec<u8>, bool)> {
    use std::io::Read;
    let mut buf = Vec::new();
    source.take(cap as u64 + 1).read_to_end(&mut buf)?;
    let overflowed = buf.len() > cap;
    buf.truncate(cap);
    Ok((buf, overflowed))
}

/// Spawns `cmd`, writes `input` to its stdin, and collects its output.
///
/// stderr is drained on its own thread rather than after stdout. Reading
/// them in sequence deadlocks whenever a child fills the stderr pipe before
/// producing stdout, which `GH_DEBUG=api` makes routine: gh streams whole
/// HTTP logs to stderr first, the parent is still blocked on an empty
/// stdout, and neither side moves again.
///
/// Writing the whole request before reading anything is still sequential,
/// and safe: both children consume the request body before answering, and
/// it is far below a pipe buffer.
fn run(mut cmd: Command, input: &[u8]) -> Result<Ran, String> {
    use std::io::Write;
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("could not run {:?}: {err}", cmd.get_program()))?;

    let mut stderr_pipe = child.stderr.take().expect("stderr piped");
    let draining = std::thread::spawn(move || read_capped(&mut stderr_pipe, MAX_STDERR));

    // Every early return from here on kills the child first: leaving it
    // running would orphan a process nobody will ever wait on, and its
    // stderr is usually the only thing that explains what went wrong.
    let fail = |child: &mut std::process::Child, message: String| {
        let _ = child.kill();
        let _ = child.wait();
        message
    };

    if let Err(err) = child
        .stdin
        .take()
        .expect("stdin piped")
        .write_all(input)
        .map_err(|err| format!("could not send the request: {err}"))
    {
        return Err(fail(&mut child, err));
    }

    let mut stdout_pipe = child.stdout.take().expect("stdout piped");
    let (stdout, too_big) = match read_capped(&mut stdout_pipe, MAX_RESPONSE) {
        Ok(read) => read,
        Err(err) => {
            return Err(fail(
                &mut child,
                format!("could not read the response: {err}"),
            ))
        }
    };
    if too_big {
        return Err(fail(
            &mut child,
            format!(
                "the GitHub API response exceeded {} MiB",
                MAX_RESPONSE / (1024 * 1024)
            ),
        ));
    }

    // The draining thread stops at MAX_STDERR, so a child still writing
    // past it would block forever on a full pipe; killing it first makes
    // `wait` return. Under the cap the pipe is already at EOF and the kill
    // is a no-op on an exited child.
    let stderr = match draining.join() {
        Ok(Ok((bytes, overflowed))) => {
            if overflowed {
                let _ = child.kill();
            }
            bytes
        }
        _ => Vec::new(),
    };
    let status = child
        .wait()
        .map_err(|err| format!("could not read the response: {err}"))?;
    Ok(Ran {
        status,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

/// The repository behind a bare `#123`, from the git remote.
///
/// A fork checkout has two plausible answers, and picking one silently is
/// the failure this tool exists to avoid: `#123` would resolve against
/// somebody else's tracker without a word. So a disagreement is reported
/// and `repo` in the config is named as the way to settle it.
fn default_repo(dir: &Path) -> Result<Repo, String> {
    const FIX: &str = "set repo = \"owner/name\" in todo-by.toml";
    let origin = remote_repo(dir, "origin");
    let upstream = remote_repo(dir, "upstream");
    match (origin, upstream) {
        (Some(origin), Some(upstream)) if origin != upstream => Err(format!(
            "#N is ambiguous here: origin is {}/{} but upstream is {}/{}; {FIX}",
            origin.owner, origin.name, upstream.owner, upstream.name
        )),
        (Some(repo), _) | (None, Some(repo)) => Ok(repo),
        (None, None) => Err(format!(
            "could not determine the repository for #N from a git remote; {FIX}"
        )),
    }
}

fn remote_repo(dir: &Path, name: &str) -> Option<Repo> {
    let output = Command::new("git")
        .args(["remote", "get-url", name])
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_remote_url(String::from_utf8_lossy(&output.stdout).trim())
}

/// Parses the remote spellings git produces: `scheme://[user@]host[:port]/
/// owner/repo[.git]`, and the scp-like `[user@]host:owner/repo[.git]`.
///
/// The presence of a scheme is what tells the two apart, and nothing else
/// does reliably: a colon means a port in the first and the start of the
/// path in the second, so guessing from the text around it gets
/// `git@github.com:user2/repo.git` wrong the moment an owner ends in a
/// digit, exactly as a port does.
///
/// The last two path segments are the repository, which is what makes this
/// work on a GitHub Enterprise instance served from a subpath.
pub fn parse_remote_url(url: &str) -> Option<Repo> {
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (Some(scheme), rest),
        None => (None, url),
    };
    let rest = rest.rsplit_once('@').map_or(rest, |(_, after)| after);
    let (host, path) = match scheme {
        Some(_) => rest.split_once('/')?,
        None => rest.split_once(':')?,
    };
    // The API is reached over HTTPS, so a port belonging to another
    // protocol must not travel with the host: `ssh://host:2222/` says
    // nothing about where the web API listens. A port on an http(s) remote
    // is kept, since there it does.
    // A port on an http(s) remote is where the API listens, so it stays
    // (minus the default). Any other scheme's port belongs to another
    // protocol: `ssh://host:2222/` says nothing about the web API.
    let host = match scheme {
        Some("http" | "https") => host,
        _ => host.split(':').next()?,
    };
    build_repo(host, path)
}

fn build_repo(host: &str, path: &str) -> Option<Repo> {
    let host = normalize_host(host);
    let path = path.strip_suffix(".git").unwrap_or(path);
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let [.., owner, name] = segments[..] else {
        return None;
    };
    if !valid_host(&host) {
        return None;
    }
    Repo::parse_slug(&format!("{owner}/{name}"), &host)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(host: &str, owner: &str, name: &str) -> Repo {
        Repo {
            host: host.to_string(),
            owner: owner.to_string(),
            name: name.to_string(),
        }
    }

    #[test]
    fn parses_the_hash_form() {
        assert_eq!(
            Reference::parse("#123"),
            Some(Reference {
                repo: None,
                number: 123
            })
        );
    }

    #[test]
    fn parses_issue_and_pull_urls() {
        for (url, expected) in [
            ("https://github.com/alies-dev/todo-by/issues/7", 7),
            ("https://github.com/alies-dev/todo-by/pull/7", 7),
            ("http://github.com/alies-dev/todo-by/issues/7", 7),
        ] {
            assert_eq!(
                Reference::parse(url),
                Some(Reference {
                    repo: Some(repo("github.com", "alies-dev", "todo-by")),
                    number: expected
                }),
                "{url}"
            );
        }
        assert_eq!(
            Reference::parse("https://git.acme.corp/team/app/issues/4"),
            Some(Reference {
                repo: Some(repo("git.acme.corp", "team", "app")),
                number: 4
            })
        );
    }

    #[test]
    fn rejects_shapes_that_are_not_references() {
        for bad in [
            "#",
            "#0",
            "#12x",
            "#99999999",
            "#1.2",
            "owner/repo#1",
            "https://github.com/owner/repo",
            "https://github.com/owner/repo/commits/1",
            "https://github.com/owner/repo/issues/x",
            "https://github.com/owner/repo/issues/1/extra",
            "https://github.com/ow ner/repo/issues/1",
        ] {
            assert_eq!(Reference::parse(bad), None, "expected {bad:?} rejected");
        }
    }

    #[test]
    fn parses_every_remote_spelling() {
        for url in [
            "https://github.com/alies-dev/todo-by.git",
            "https://github.com/alies-dev/todo-by",
            "https://alies@github.com/alies-dev/todo-by.git",
            "git@github.com:alies-dev/todo-by.git",
            "ssh://git@github.com/alies-dev/todo-by.git",
            "git://github.com/alies-dev/todo-by.git",
        ] {
            assert_eq!(
                parse_remote_url(url),
                Some(repo("github.com", "alies-dev", "todo-by")),
                "{url}"
            );
        }
        assert_eq!(parse_remote_url("not-a-url"), None);
        assert_eq!(
            parse_remote_url("https://github.com/only-one-segment"),
            None
        );
    }

    #[test]
    fn an_owner_ending_in_a_digit_is_not_mistaken_for_a_port() {
        // The colon in scp-like syntax starts the path; in a real URL it
        // starts a port. Only the scheme separates the two cases.
        assert_eq!(
            parse_remote_url("git@github.com:user2/repo.git"),
            Some(repo("github.com", "user2", "repo"))
        );
        assert_eq!(
            parse_remote_url("git@github.com:2fa/repo.git"),
            Some(repo("github.com", "2fa", "repo"))
        );
    }

    #[test]
    fn an_ssh_port_does_not_follow_the_host_to_the_api() {
        // The web API does not listen on the SSH port, so it is dropped.
        assert_eq!(
            parse_remote_url("ssh://git@git.acme.corp:2222/team/app.git"),
            Some(repo("git.acme.corp", "team", "app"))
        );
        // An https port is where the API actually is, so it is kept.
        assert_eq!(
            parse_remote_url("https://git.acme.corp:8443/team/app.git"),
            Some(repo("git.acme.corp:8443", "team", "app"))
        );
        assert_eq!(
            repo("git.acme.corp:8443", "team", "app").graphql_url(),
            "https://git.acme.corp:8443/api/graphql"
        );
    }

    #[test]
    fn default_ports_and_the_ssh_over_https_alias_normalize_to_github_com() {
        for url in [
            "https://github.com:443/alies-dev/todo-by.git",
            "http://github.com:80/alies-dev/todo-by.git",
            "ssh://git@ssh.github.com:443/alies-dev/todo-by.git",
        ] {
            assert_eq!(
                parse_remote_url(url),
                Some(repo("github.com", "alies-dev", "todo-by")),
                "{url}"
            );
        }
        assert_eq!(
            parse_remote_url("https://github.com/alies-dev/todo-by.git")
                .unwrap()
                .graphql_url(),
            "https://api.github.com/graphql"
        );
    }

    #[test]
    fn a_subpath_hosted_instance_keeps_the_last_two_segments() {
        assert_eq!(
            parse_remote_url("https://git.acme.corp/scm/team/app.git"),
            Some(repo("git.acme.corp", "team", "app"))
        );
    }

    #[test]
    fn builds_one_query_per_batch_with_stable_aliases() {
        let repos = vec![repo("github.com", "o", "r"), repo("github.com", "o", "s")];
        let (url, body) = request(&repos, &[(0, 1), (0, 2), (1, 3)]);
        assert_eq!(url, "https://api.github.com/graphql");
        assert!(body.starts_with("{\"query\":\"query {"), "{body}");
        assert!(
            body.contains(r#"r0: repository(owner: \"o\", name: \"r\")"#),
            "{body}"
        );
        assert!(body.contains("n1: issueOrPullRequest(number: 1)"), "{body}");
        assert!(body.contains("n3: issueOrPullRequest(number: 3)"), "{body}");
        assert!(
            !body.contains("stateReason"),
            "the close reason is never requested"
        );
        assert_eq!(body.matches("repository(owner:").count(), 2);
    }

    #[test]
    fn enterprise_host_gets_its_own_endpoint() {
        let repos = vec![repo("git.acme.corp", "team", "app")];
        let (url, _) = request(&repos, &[(0, 9)]);
        assert_eq!(url, "https://git.acme.corp/api/graphql");
    }

    #[test]
    fn batches_split_by_host_and_size() {
        let repos = vec![repo("github.com", "o", "r"), repo("acme.dev", "o", "r")];
        let targets: Vec<Result<(usize, u32), String>> = (0..BATCH as u32 + 5)
            .map(|n| Ok((0, n + 1)))
            .chain([Ok((1, 1)), Err("no remote".to_string())])
            .collect();
        let batches = batches(&targets, &repos);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].len(), BATCH);
        assert_eq!(batches[1].len(), 5);
        assert_eq!(batches[2], vec![(1, 1)]);
    }

    #[test]
    fn interleaved_repositories_are_grouped_so_no_alias_repeats() {
        // Source order can interleave repositories; `request` opens a new
        // rN block on every change, so an ungrouped batch would emit `r0`
        // twice in one query and the API would reject it.
        let repos = vec![repo("github.com", "o", "r"), repo("github.com", "o", "s")];
        let targets = vec![Ok((0, 1)), Ok((1, 2)), Ok((0, 3))];
        let batches = batches(&targets, &repos);
        assert_eq!(batches, vec![vec![(0, 1), (0, 3), (1, 2)]]);
        let (_, body) = request(&repos, &batches[0]);
        assert_eq!(body.matches("r0: repository").count(), 1, "{body}");
        assert_eq!(body.matches("r1: repository").count(), 1, "{body}");
        // Braces must balance: one block per repository plus the outer.
        assert_eq!(
            body.matches('{').count() - body.matches("{{").count(),
            body.matches('}').count() - body.matches("}}").count()
        );
    }

    #[test]
    fn duplicate_references_query_once() {
        let repos = vec![repo("github.com", "o", "r")];
        let targets = vec![Ok((0, 5)), Ok((0, 5)), Ok((0, 6))];
        assert_eq!(batches(&targets, &repos), vec![vec![(0, 5), (0, 6)]]);
    }

    #[test]
    fn reads_states_and_per_reference_errors() {
        let response = r#"{"data":{"r0":{"n1":{"state":"OPEN"},"n2":{"state":"MERGED"},
                           "n3":null}},"errors":[{"path":["r0","n3"],
                           "message":"Could not resolve to an Issue"}]}"#;
        let mut states = HashMap::new();
        read_response(response, &[(0, 1), (0, 2), (0, 3)], &mut states).expect("partial ok");
        assert_eq!(states[&(0, 1)], Ok("OPEN".to_string()));
        assert_eq!(states[&(0, 2)], Ok("MERGED".to_string()));
        assert_eq!(
            states[&(0, 3)],
            Err("Could not resolve to an Issue".to_string())
        );
    }

    #[test]
    fn a_repository_level_error_explains_every_reference_under_it() {
        let response = r#"{"data":{"r0":null},"errors":[{"path":["r0"],
                           "message":"Could not resolve to a Repository"}]}"#;
        let mut states = HashMap::new();
        read_response(response, &[(0, 1), (0, 2)], &mut states).expect("partial ok");
        assert_eq!(
            states[&(0, 1)],
            Err("Could not resolve to a Repository".to_string())
        );
        assert_eq!(states[&(0, 1)], states[&(0, 2)]);
    }

    #[test]
    fn a_response_with_no_data_is_a_whole_batch_failure() {
        let mut states = HashMap::new();
        let err = read_response(
            r#"{"data":null,"errors":[{"message":"Bad credentials"}]}"#,
            &[(0, 1)],
            &mut states,
        )
        .expect_err("fatal");
        assert!(err.contains("Bad credentials"), "{err}");
        assert!(states.is_empty());
    }

    #[test]
    fn an_unreadable_response_names_what_came_back() {
        let mut states = HashMap::new();
        let err = read_response("<html>proxy error</html>", &[(0, 1)], &mut states)
            .expect_err("unreadable");
        assert!(err.contains("<html>proxy error</html>"), "{err}");
    }

    #[test]
    fn syntax_help_names_the_right_spelling() {
        assert!(syntax_help("#12x").contains("#123"));
        assert!(syntax_help("https://github.com/o/r").contains("issues/123"));
    }

    #[test]
    fn an_environment_token_never_leaves_github_com() {
        // Hosts come out of scanned comments, so this is the line between
        // "read a repository" and "a comment decides where a credential
        // goes". Whatever else happens for another host, it is not Curl.
        for host in ["evil.example", "git.acme.corp", "github.com.evil.test"] {
            match choose_transport(host, Some("secret".to_string())) {
                Ok(Transport::Curl(_)) => panic!("{host} must not receive the environment token"),
                Ok(Transport::Gh) | Err(_) => {}
            }
        }
    }

    #[test]
    fn github_com_still_uses_the_token_when_curl_is_present() {
        if !have("curl") {
            return;
        }
        assert!(matches!(
            choose_transport("github.com", Some("secret".to_string())),
            Ok(Transport::Curl(token)) if token == "secret"
        ));
    }

    #[test]
    fn each_missing_reference_gets_its_own_message() {
        let response = r#"{"data":{"r0":{"n3":null,"n5":null}},"errors":[
            {"path":["r0","n3"],"message":"no issue 3"},
            {"path":["r0","n5"],"message":"no issue 5"}]}"#;
        let mut states = HashMap::new();
        read_response(response, &[(0, 3), (0, 5)], &mut states).expect("partial ok");
        assert_eq!(states[&(0, 3)], Err("no issue 3".to_string()));
        assert_eq!(states[&(0, 5)], Err("no issue 5".to_string()));
    }

    #[test]
    fn excerpt_cuts_on_a_character_boundary() {
        // A proxy error page with a multi-byte character straddling the cut
        // would panic a byte slice, on the path whose only job is reporting
        // somebody else's garbage.
        let line = format!("{}é tail", "x".repeat(199));
        let cut = excerpt(&line);
        assert!(cut.ends_with("..."), "{cut}");
        assert!(cut.len() <= 203);
    }

    #[test]
    fn a_comment_permalink_still_names_its_issue() {
        for url in [
            "https://github.com/o/r/issues/123#issuecomment-456",
            "https://github.com/o/r/issues/123?foo=1",
            "https://github.com/o/r/pull/123#discussion_r1",
        ] {
            assert_eq!(
                Reference::parse(url),
                Some(Reference {
                    repo: Some(repo("github.com", "o", "r")),
                    number: 123
                }),
                "{url}"
            );
        }
    }

    #[test]
    fn a_url_in_a_comment_gets_the_same_host_normalization_as_a_remote() {
        assert_eq!(
            Reference::parse("https://GitHub.com:443/o/r/issues/1")
                .and_then(|r| r.repo)
                .map(|r| r.host),
            Some("github.com".to_string())
        );
    }

    #[test]
    fn a_token_with_a_line_break_is_refused_before_anything_spawns() {
        let err =
            send_curl("tok\nen", "https://api.github.com/graphql", "{}").expect_err("refused");
        assert!(err.contains("line break"), "{err}");
    }

    #[test]
    fn references_with_no_resolvable_repository_all_fail_without_a_request() {
        // A directory outside any repository: every bare `#N` fails to
        // resolve, so there is nothing to batch and no transport is ever
        // probed. Hermetic, which is why it is the one resolve() path
        // testable without a network.
        let dir = std::env::temp_dir().join(format!("todo-by-resolve-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let refs = vec![
            Reference {
                repo: None,
                number: 1,
            },
            Reference {
                repo: None,
                number: 2,
            },
        ];
        let (outcomes, failure) = resolve(&refs, None, &dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(failure.is_none(), "no request was attempted");
        assert_eq!(outcomes.len(), 2);
        for outcome in outcomes {
            match outcome {
                Outcome::Failed(detail) => assert!(detail.contains("repo ="), "{detail}"),
                _ => panic!("expected every reference to fail"),
            }
        }
    }

    #[test]
    fn control_characters_from_a_server_never_reach_the_terminal() {
        let hostile = "body\u{1b}[2Jwiped";
        assert_eq!(sanitize(hostile), "body [2Jwiped");
        assert!(!excerpt(hostile).contains('\u{1b}'));
    }

    #[test]
    fn curl_config_values_are_escaped() {
        assert_eq!(escape_curl(r#"a"b\c"#), r#"a\"b\\c"#);
    }

    #[test]
    fn staged_bodies_do_not_collide() {
        let first = stage_body(r#"{"query":"a"}"#).expect("staged");
        let second = stage_body(r#"{"query":"b"}"#).expect("staged");
        assert_ne!(first, second);
        assert_eq!(std::fs::read_to_string(&first).unwrap(), r#"{"query":"a"}"#);
        assert_eq!(
            std::fs::read_to_string(&second).unwrap(),
            r#"{"query":"b"}"#
        );
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);
    }

    #[test]
    fn excerpt_truncates_a_long_line() {
        let long = "x".repeat(500);
        assert_eq!(excerpt(&long).len(), 203);
        assert_eq!(excerpt("\n\nfirst\nsecond"), "first");
    }
}
