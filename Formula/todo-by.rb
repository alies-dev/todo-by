# This repo doubles as a Homebrew tap. Users install with:
#   brew tap alies-dev/todo-by https://github.com/alies-dev/todo-by
#   brew install alies-dev/todo-by/todo-by
#
# This file is updated automatically by the homebrew-tap workflow.

class TodoBy < Formula
  desc "Flag todo-by tags whose deadline date has passed, across any file type"
  homepage "https://github.com/alies-dev/todo-by"
  version "0.5.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/alies-dev/todo-by/releases/download/v#{version}/todo-by-cli-aarch64-apple-darwin.tar.xz"
      sha256 "ae1a4eace9dfdfd1feec17fa85878ea075bf0a028dfc9630b08750d0aa247f36"
    end
    on_intel do
      url "https://github.com/alies-dev/todo-by/releases/download/v#{version}/todo-by-cli-x86_64-apple-darwin.tar.xz"
      sha256 "331dbea9d4c20f903a9a14e1255bd8b1c69dd2e0623b4b2f68aff30cea3bc166"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/alies-dev/todo-by/releases/download/v#{version}/todo-by-cli-x86_64-unknown-linux-gnu.tar.xz"
      sha256 "043c53922cc0692146802b1de3528d2a39ebdeb395fadfa4c5f38a42c5ca084e"
    end
  end

  def install
    bin.install "todo-by"
  end

  test do
    assert_match "todo-by", shell_output("#{bin}/todo-by --help")
  end
end
