class AgentMem < Formula
  desc "Ultra-fast, local-first memory engine for AI coding agents"
  homepage "https://github.com/marcosotomac/agent-mem"
  version "0.1.4"
  license "MIT"

  if OS.mac? && Hardware::CPU.arm?
    url "https://github.com/marcosotomac/agent-mem/releases/download/v#{version}/agent-mem-darwin-aarch64.tar.gz"
    # sha256 checksum populated on release
  elsif OS.mac? && Hardware::CPU.intel?
    url "https://github.com/marcosotomac/agent-mem/releases/download/v#{version}/agent-mem-darwin-x86_64.tar.gz"
  elsif OS.linux? && Hardware::CPU.intel?
    url "https://github.com/marcosotomac/agent-mem/releases/download/v#{version}/agent-mem-linux-x86_64.tar.gz"
  elsif OS.linux? && Hardware::CPU.arm?
    url "https://github.com/marcosotomac/agent-mem/releases/download/v#{version}/agent-mem-linux-aarch64.tar.gz"
  end

  def install
    bin.install "agent-mem"
  end

  test do
    system "#{bin}/agent-mem", "--help"
  end
end
