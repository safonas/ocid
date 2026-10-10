# MkDocs macros for ocid documentation
# Usage: {{ macros.version }} or {{ macros.latest_release }}

from datetime import datetime
import os

# Get the current version from Cargo.toml
def get_version():
    # mkdocs runs from docs/, the workspace Cargo.toml sits one level up
    for candidate in ("Cargo.toml", "../Cargo.toml"):
        try:
            with open(candidate, "r") as f:
                for line in f:
                    if line.strip().startswith("version"):
                        return line.split("=")[1].strip().strip('"').strip("'")
        except FileNotFoundError:
            continue
    return "unknown"

# Get the latest release from GitHub (fallback to version)
def get_latest_release():
    version = get_version()
    # Remove alpha/beta/rc suffixes for stable releases
    if "-alpha" in version or "-beta" in version or "-rc" in version:
        return version
    return version

# Get the current year
def get_year():
    return datetime.now().year

# Get the GitHub URL
def get_github_url():
    return "https://github.com/safonas/ocid"

# Get the docs URL
def get_docs_url():
    return "https://ocid.dev"

# Define macros
def define_env(env):
    @env.macro
    def version():
        return get_version()

    @env.macro
    def latest_release():
        return get_latest_release()

    @env.macro
    def year():
        return get_year()

    @env.macro
    def github_url():
        return get_github_url()

    @env.macro
    def docs_url():
        return get_docs_url()

    @env.macro
    def ocid_home():
        return "$OCID_HOME"

    @env.macro
    def registry_url():
        return "127.0.0.1:5050"

    @env.macro
    def control_api_url():
        return "127.0.0.1:5050/_ocid"

    @env.macro
    def metrics_url():
        return "127.0.0.1:5050/metrics"
