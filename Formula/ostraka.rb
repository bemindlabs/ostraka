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
  version "1.7.3"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/bemindlabs/ostraka/releases/download/v#{version}/ostraka-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "18ee727de78bde60b5c831108c0c86b30357d9a4ab7390e1e2b2862bafdd8e59"
    end
    on_intel do
      url "https://github.com/bemindlabs/ostraka/releases/download/v#{version}/ostraka-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "22f83179b1ff3d4978b676e386ca31be051435cbbc0a2c397c4a936a61e1f767"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/bemindlabs/ostraka/releases/download/v#{version}/ostraka-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "6e94b40731dfaccf07ed7470f9b82dee7eb0ef31e39376c5d473dfa3c38a1fd7"
    end
    on_intel do
      url "https://github.com/bemindlabs/ostraka/releases/download/v#{version}/ostraka-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "4a5e15852e0f26b71e1b15aaacd6a744cd9ca886fe72dcc5c90382340a92487a"
    end
  end

  def install
    bin.install "ostraka"
  end

  test do
    assert_match "ostraka", shell_output("#{bin}/ostraka --version")
  end
end
