# Homebrew formula for ocid — the single source of truth. `just packaging
# brew` (audit) and `just ship publish-release` (tap bump) copy this file
# into the tap (Formula/ocid.rb) and rewrite url/sha256 in place; only
# those two lines should ever differ from the tap's copy.
class Ocid < Formula
  desc "Local-first, peer-to-peer distribution of OCI container images powered by iroh"
  homepage "https://github.com/safonas/ocid"
  url "https://github.com/safonas/ocid/archive/refs/tags/v0.6.2.tar.gz"
  sha256 "5a6db8976ee495e2649d51f13f20d931fdad04aed66d00e8f16784b6ad1e704e"
  license "GPL-3.0-or-later"
  head "https://github.com/safonas/ocid.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/ocid")
    system "cargo", "install", *std_cargo_args(path: "crates/ocictl")
    system "cargo", "install", *std_cargo_args(path: "crates/ocitop")
  end

  # brew services start ocid — a launchd agent on macOS, a systemd user
  # service on Linux. Same shape as every other channel: TLS on
  # 127.0.0.1:5050 (trust the CA at ~/.ocid/tls/ca.crt), state in the
  # user's default OCID_HOME.
  service do
    run [opt_bin/"ocid", "--tls"]
    keep_alive true
    log_path var/"log/ocid.log"
    error_log_path var/"log/ocid.log"
  end

  test do
    assert_match "ocid", shell_output("#{bin}/ocid --version")
    assert_match "ocictl", shell_output("#{bin}/ocictl --version")
  end
end
