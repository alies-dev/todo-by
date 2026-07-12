# This repo doubles as a Homebrew tap. Users install with:
#   brew tap alies-dev/todo-by https://github.com/alies-dev/todo-by
#   brew install alies-dev/todo-by/todo-by
#
# This file is updated automatically by the homebrew-tap workflow.

class TodoBy < Formula
  desc "Flag todo-by tags whose deadline date has passed, across any file type"
  homepage "https://github.com/alies-dev/todo-by"
  version "0.2.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/alies-dev/todo-by/releases/download/v#{version}/todo-by-cli-aarch64-apple-darwin.tar.xz"
      sha256 "605312ca35c49ddc7c25fc5873a3922d3426048827f5db7b85ee217b6da37e3c"
    end
    on_intel do
      url "https://github.com/alies-dev/todo-by/releases/download/v#{version}/todo-by-cli-x86_64-apple-darwin.tar.xz"
      sha256 "0cf1cc5f7a103b4c1b342c3c0f877b86dde5d214c86e08f4c54d6cd4dbebde36"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/alies-dev/todo-by/releases/download/v#{version}/todo-by-cli-x86_64-unknown-linux-gnu.tar.xz"
      sha256 "95887668cf8960e54935f3ba13c0a57829a856612d9a300b87a33e40b97cdb03"
    end
  end

  def install
    bin.install "todo-by"
  end

  test do
    assert_match "todo-by", shell_output("#{bin}/todo-by --help")
  end
end
