# This repo doubles as a Homebrew tap. Users install with:
#   brew tap alies-dev/todo-by https://github.com/alies-dev/todo-by
#   brew install alies-dev/todo-by/todo-by
#
# This file is updated automatically by the homebrew-tap workflow.

class TodoBy < Formula
  desc "Flag todo-by tags whose deadline date has passed, across any file type"
  homepage "https://github.com/alies-dev/todo-by"
  version "0.2.1"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/alies-dev/todo-by/releases/download/v#{version}/todo-by-cli-aarch64-apple-darwin.tar.xz"
      sha256 "46b344c1a617736a9fb4bee9f77d29d51c9cd99efc1ca3aa8d4aaafdbda9801c"
    end
    on_intel do
      url "https://github.com/alies-dev/todo-by/releases/download/v#{version}/todo-by-cli-x86_64-apple-darwin.tar.xz"
      sha256 "151ab6a57d88533cec092a74997685d3fe41b671d5df315a5c018284b00475a1"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/alies-dev/todo-by/releases/download/v#{version}/todo-by-cli-x86_64-unknown-linux-gnu.tar.xz"
      sha256 "bb09fb66db08eb31dcd1fd9853cc23162106ca3a2f5d32b4e487925a3337eb8e"
    end
  end

  def install
    bin.install "todo-by"
  end

  test do
    assert_match "todo-by", shell_output("#{bin}/todo-by --help")
  end
end
