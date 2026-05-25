class AitNative < Formula
  preserve_rpath

  desc "Agent-first Markdown workflow CLI with optional self-hosted server and worker surfaces"
  homepage "https://ait-native.dev"
  url "https://files.pythonhosted.org/packages/27/02/1a3271e0066aa56d03d64cf24f719ba2797496e3fc852ae2c87bf7230f33/ait_native-0.10.3-py3-none-any.whl"
  sha256 "0ffc35a66dfc99304d7545b43a4cb3a42187eff99026bf9277ceb75d3d339319"
  license all_of: ["Apache-2.0", "AGPL-3.0-only"]

  depends_on "python@3.14"

  def install
    system Formula["python@3.14"].opt_bin/"python3", "-m", "venv", libexec
    wheel = buildpath/"ait_native-0.10.3-py3-none-any.whl"
    cp cached_download, wheel
    system libexec/"bin/python", "-m", "pip", "install", wheel
    bin.install_symlink libexec/"bin/ait"
    bin.install_symlink libexec/"bin/ait-agent"
    bin.install_symlink libexec/"bin/ait-server"
    bin.install_symlink libexec/"bin/ait-worker"
    bin.install_symlink libexec/"bin/aitk"
  end

  def caveats
    <<~EOS
      Homebrew tap formula for the public `ait-native` package.
      This formula intentionally avoids auto-starting services.
      `ait-server` and `ait-worker` still require self-hosted runtime configuration.
    EOS
  end
end
