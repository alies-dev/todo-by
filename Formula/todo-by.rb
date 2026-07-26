# This repo doubles as a Homebrew tap. Users install with:
#   brew tap alies-dev/todo-by https://github.com/alies-dev/todo-by
#   brew install alies-dev/todo-by/todo-by
#
# This file is updated automatically by the homebrew-tap workflow.

class TodoBy < Formula
  desc "Flag todo-by tags whose deadline date has passed, across any file type"
  homepage "https://github.com/alies-dev/todo-by"
  version "0.3.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/alies-dev/todo-by/releases/download/v#{version}/todo-by-cli-aarch64-apple-darwin.tar.xz"
      sha256 "7af7fb024d8b9a14d27bb1c776fcc799ab1838cb95f57b63d3df0b0af1f19fc5"
    end
    on_intel do
      url "https://github.com/alies-dev/todo-by/releases/download/v#{version}/todo-by-cli-x86_64-apple-darwin.tar.xz"
      sha256 "333f4fb036f140062f4a6e4393ed9839860e7627aaf633508b449daa163d6b28"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/alies-dev/todo-by/releases/download/v#{version}/todo-by-cli-x86_64-unknown-linux-gnu.tar.xz"
      sha256 "fa4a2b237bd62b7f3541a7dbafb1f9305ad58e0d4d117855ed38dc15cf0756f4"
    end
  end

  def install
    bin.install "todo-by"
  end

  test do
    assert_match "todo-by", shell_output("#{bin}/todo-by --help")
  end
end
