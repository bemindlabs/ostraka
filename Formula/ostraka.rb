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
  version "1.5.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/bemindlabs/ostraka/releases/download/v#{version}/ostraka-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "7d55e4b91704f6161ecd3facd20c4ded767c7cbaed50c49e360976e2f7e8ab2c"
    end
    on_intel do
      url "https://github.com/bemindlabs/ostraka/releases/download/v#{version}/ostraka-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "d4bb22c36793ed4ab484d412a38f37fc3f38e915459d9d484efc8a7699350533"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/bemindlabs/ostraka/releases/download/v#{version}/ostraka-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "26f5d31f69bb1d9ea8af9f9b8fd320cb920c9352b0c674658ff857f5cb9d89ba"
    end
    on_intel do
      url "https://github.com/bemindlabs/ostraka/releases/download/v#{version}/ostraka-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "96f5e41dc436fcdefe34e1c7633b9164fe8a2aba73c4846354be0d7861749498"
    end
  end

  def install
    bin.install "ostraka"
  end

  test do
    assert_match "ostraka", shell_output("#{bin}/ostraka --version")
  end
end
