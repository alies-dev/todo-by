# This repo doubles as a Homebrew tap. Users install with:
#   brew tap alies-dev/todo-by https://github.com/alies-dev/todo-by
#   brew install alies-dev/todo-by/todo-by
#
# This file is updated automatically by the homebrew-tap workflow.

class TodoBy < Formula
  desc "Flag todo-by tags whose deadline date has passed, across any file type"
  homepage "https://github.com/alies-dev/todo-by"
  version "0.4.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/alies-dev/todo-by/releases/download/v#{version}/todo-by-cli-aarch64-apple-darwin.tar.xz"
      sha256 "f0f4b73903be638f3ef81b50b9cb35d02d47534012ed2ff63f371db340ce5687"
    end
    on_intel do
      url "https://github.com/alies-dev/todo-by/releases/download/v#{version}/todo-by-cli-x86_64-apple-darwin.tar.xz"
      sha256 "7375915a127ec289f6c2b826831dd6a9d13d829ccb3ee539533055e365325377"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/alies-dev/todo-by/releases/download/v#{version}/todo-by-cli-x86_64-unknown-linux-gnu.tar.xz"
      sha256 "69c6dad6114a149f35bbd15660c4d1ec8e78858a4df7f03bc2ce92bf4f1b4487"
    end
  end

  def install
    bin.install "todo-by"
  end

  test do
    assert_match "todo-by", shell_output("#{bin}/todo-by --help")
  end
end
