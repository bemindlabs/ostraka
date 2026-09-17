# Homebrew formula, kept in-repo so a tap needs no second repository — which
# works, but only when the tap is given this repository's URL:
#
#   brew tap bemindlabs/ostraka https://github.com/bemindlabs/ostraka
#
# `brew tap user/repo` on its own resolves to `github.com/user/homebrew-repo`.
# The prefix is brew's, not the user's, so the bare two-argument form looks for
# a repository that does not exist. The README carries the URL form for that
# reason; changing one without the other breaks installing from a tap.
#
# The release workflow rewrites the version and the sha256 fields; the artifact
# name it parses is fixed by that workflow.
class Ostraka < Formula
  desc "Run agent fleets you can actually review"
  homepage "https://github.com/bemindlabs/ostraka"
  version "1.6.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/bemindlabs/ostraka/releases/download/v#{version}/ostraka-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "340d60fca1c600f0cad25e27bffe5a5a657e4d150cee3c6ad8548d265d1c9caf"
    end
    on_intel do
      url "https://github.com/bemindlabs/ostraka/releases/download/v#{version}/ostraka-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "a0436c37376dc44a5fac76cd3ad41c07a1dcb8cccbfe8f98bd69306f0a03d99b"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/bemindlabs/ostraka/releases/download/v#{version}/ostraka-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "05701e57a81ec6aa2f298cf8a72c87eb3e78aaa18d2ad61c0b1b7928cc050f57"
    end
    on_intel do
      url "https://github.com/bemindlabs/ostraka/releases/download/v#{version}/ostraka-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "fcfedcd788f3ee4b28b6ba631734b10ddc3d383912560b97c92814fb592730b5"
    end
  end

  def install
    bin.install "ostraka"
  end

  test do
    assert_match "ostraka", shell_output("#{bin}/ostraka --version")
  end
end
