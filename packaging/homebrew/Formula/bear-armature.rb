class BearArmature < Formula
  desc "BEARS ACP stdio adapter for Zed and other ACP clients"
  homepage "https://github.com/bears-ai/bear-den"
  version "0.1.0"
  # License: see LICENSE in the upstream repo once added.

  on_macos do
    on_arm do
      url "https://github.com/bears-ai/bear-den/releases/download/bear-armature%2Fv#{version}/bear-armature-aarch64-apple-darwin.tar.gz"
      sha256 "" # fill in from `sha256sum` output printed by the release workflow
    end

    on_intel do
      url "https://github.com/bears-ai/bear-den/releases/download/bear-armature%2Fv#{version}/bear-armature-x86_64-apple-darwin.tar.gz"
      sha256 "" # fill in from `sha256sum` output printed by the release workflow
    end
  end

  def install
    bin.install "bear-armature"
    bin.install_symlink "bear-armature" => "bears-acp-adapter"
  end

  test do
    # --help exits 0 and prints usage to stderr
    system "#{bin}/bear-armature", "--help"
  end
end
