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
  version "1.0.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/bemindlabs/ostraka/releases/download/v#{version}/ostraka-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "c4a997d2603492f663482fdee6e5ab2bc06448c3422dfa601c41bcfae0dd349e"
    end
    on_intel do
      url "https://github.com/bemindlabs/ostraka/releases/download/v#{version}/ostraka-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "115627b8642603a583dc36d4e1fd6de6ac0a6fe310939aed4e88667cf0adf584"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/bemindlabs/ostraka/releases/download/v#{version}/ostraka-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "5d42e0a5f9de822a99a4497bd9e97b0106ad89e3a4b19d3f072e69e15e47c2a3"
    end
    on_intel do
      url "https://github.com/bemindlabs/ostraka/releases/download/v#{version}/ostraka-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "bd528a8c99f2116ce8e24d6063391fdc0094e7a22a2514fe3381bcba1ac0850a"
    end
  end

  def install
    bin.install "ostraka"
  end

  test do
    assert_match "ostraka", shell_output("#{bin}/ostraka --version")
  end
end
