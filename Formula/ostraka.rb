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
  version "1.7.1"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/bemindlabs/ostraka/releases/download/v#{version}/ostraka-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "459cdaac8cef9a18238d22b34cf4560b5877d70e20448e731c53442f026ae739"
    end
    on_intel do
      url "https://github.com/bemindlabs/ostraka/releases/download/v#{version}/ostraka-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "a95f8c3081fd05fd3514afcf4e0e2e2965a20ee51cc5d294f78c4fdcbf051c17"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/bemindlabs/ostraka/releases/download/v#{version}/ostraka-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "635bb772052a5b9c06a520e5b4e673995adc7e52520718e12edc4d1ee116fbcc"
    end
    on_intel do
      url "https://github.com/bemindlabs/ostraka/releases/download/v#{version}/ostraka-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "1fac5f21befefc10f08e618fafccc89085b7165c5bb32606481da348295197f4"
    end
  end

  def install
    bin.install "ostraka"
  end

  test do
    assert_match "ostraka", shell_output("#{bin}/ostraka --version")
  end
end
