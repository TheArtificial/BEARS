# Legacy formula name — installs bear-armature and symlinks bears-acp-adapter.
class BearsAcpAdapter < Formula
  desc "BEARS ACP stdio adapter for Zed and other ACP clients (legacy formula name)"
  homepage "https://github.com/bears-ai/bear-den"
  version "0.1.0"

  on_macos do
    on_arm do
      url "https://github.com/bears-ai/bear-den/releases/download/bear-armature%2Fv#{version}/bear-armature-aarch64-apple-darwin.tar.gz"
      sha256 ""
    end

    on_intel do
      url "https://github.com/bears-ai/bear-den/releases/download/bear-armature%2Fv#{version}/bear-armature-x86_64-apple-darwin.tar.gz"
      sha256 ""
    end
  end

  def install
    bin.install "bear-armature"
    bin.install_symlink "bear-armature" => "bears-acp-adapter"
  end

  test do
    system "#{bin}/bear-armature", "--help"
  end
end
