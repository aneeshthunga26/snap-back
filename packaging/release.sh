#!/usr/bin/env bash
# Build binary and corresponding-source archives. Run this on Linux x86_64.
set -euo pipefail

project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_root"

requested_version=
skip_build=false
for argument in "$@"; do
    case "$argument" in
        --skip-build) skip_build=true ;;
        -h|--help)
            echo 'Usage: packaging/release.sh [VERSION] [--skip-build]'
            echo 'Builds release binaries, then writes binary/source tarballs and SHA256SUMS to dist/.'
            exit 0
            ;;
        *)
            [[ -z "$requested_version" ]] || { echo 'Only one version may be supplied.' >&2; exit 1; }
            requested_version=${argument#v}
            ;;
    esac
done

[[ $(uname -s) == Linux && $(uname -m) == x86_64 ]] || {
    echo 'Binary releases currently target Linux x86_64.' >&2
    exit 1
}
[[ -z ${CARGO_BUILD_TARGET:-} || $CARGO_BUILD_TARGET == x86_64-unknown-linux-gnu ]] || {
    echo 'CARGO_BUILD_TARGET must be x86_64-unknown-linux-gnu for this release.' >&2
    exit 1
}
for required in LICENSE README.md docs/INSTALL.md Cargo.lock; do
    [[ -f "$required" ]] || { printf 'Missing release file: %s\n' "$required" >&2; exit 1; }
done

metadata=$(cargo metadata --locked --no-deps --format-version 1)
workspace_version=$(python -c '
import json, sys
data = json.loads(sys.argv[1])
versions = {p["version"] for p in data["packages"] if p["id"] in data["workspace_members"]}
if len(versions) != 1:
    raise SystemExit("All workspace packages must have the same release version")
print(versions.pop())
' "$metadata")
release_version=${requested_version:-$workspace_version}
[[ "$release_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || {
    echo 'Use a semantic version such as 0.1.0 or v0.1.0.' >&2
    exit 1
}
[[ "$release_version" == "$workspace_version" ]] || {
    printf 'Requested version %s does not match Cargo version %s.\n' "$release_version" "$workspace_version" >&2
    exit 1
}

if ! "$skip_build"; then
    cargo build --locked --release --workspace
fi
target_dir=$(python -c 'import json,sys; print(json.loads(sys.argv[1])["target_directory"])' "$metadata")
binary_dir="$target_dir/${CARGO_BUILD_TARGET:+$CARGO_BUILD_TARGET/}release"
for binary in snap-back snap-back-gui; do
    [[ -x "$binary_dir/$binary" ]] || { printf 'Missing release binary: %s\n' "$binary_dir/$binary" >&2; exit 1; }
done
[[ $("$binary_dir/snap-back" --version) == "snap-back-cli $release_version" ]] || {
    echo 'The CLI binary version does not match this release; rebuild without --skip-build.' >&2
    exit 1
}

archive_epoch=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || printf '0')}
[[ "$archive_epoch" =~ ^[0-9]+$ ]] || { echo 'SOURCE_DATE_EPOCH must be a Unix timestamp.' >&2; exit 1; }
source_revision=${SOURCE_REVISION:-$(git rev-parse --verify HEAD 2>/dev/null || printf 'unversioned')}
source_state=unversioned
if [[ -d .git || -f .git ]]; then
    source_state=clean
    if [[ -n $(git status --porcelain --untracked-files=normal -- . ':!dist' ':!target') ]]; then
        source_state=modified
    fi
fi
dist_dir="$project_root/dist"
mkdir -p "$dist_dir"
stage=$(mktemp -d "$dist_dir/.release.XXXXXX")
trap 'rm -rf -- "$stage"' EXIT
binary_name="snap-back-$release_version-linux-x86_64"
source_name="snap-back-$release_version-source"
binary_root="$stage/$binary_name"
source_root="$stage/$source_name"
mkdir -p "$binary_root/bin" "$binary_root/packaging/initcpio" "$source_root"
install -m755 "$binary_dir/snap-back" "$binary_root/bin/snap-back"
install -m755 "$binary_dir/snap-back-gui" "$binary_root/bin/snap-back-gui"
install -m755 packaging/install-tarball.sh "$binary_root/install.sh"
install -m644 LICENSE README.md "$binary_root/"
install -m644 docs/INSTALL.md "$binary_root/INSTALL.md"
cp -a docs "$binary_root/docs"
install -m644 packaging/snap-back.service packaging/snap-back.timer \
    packaging/org.snapback.manage.policy packaging/snap-back.desktop "$binary_root/packaging/"
install -m644 packaging/initcpio/* "$binary_root/packaging/initcpio/"
cat >"$binary_root/BUILD-INFO" <<EOF
Version: $release_version
Target: x86_64-unknown-linux-gnu
Source revision: $source_revision
Source tree: $source_state
Rust: $(rustc --version)
Cargo: $(cargo --version)
Build libc: $(getconf GNU_LIBC_VERSION)
EOF

# Include tracked and unignored source files, including local changes during
# packaging development. Also support building an extracted source archive.
if [[ -d .git || -f .git ]]; then
    git ls-files --cached --others --exclude-standard -z -- . ':!dist' ':!target' >"$stage/source-files"
else
    find . \( -path './.git' -o -path './dist' -o -path './target' \) -prune \
        -o \( -type f -o -type l \) -print0 >"$stage/source-files"
fi
tar --null -C "$project_root" -T "$stage/source-files" -cf - | tar -C "$source_root" -xf -
install -m644 "$binary_root/BUILD-INFO" "$source_root/BUILD-INFO"

# The corresponding source includes the exact dependency sources and an offline
# Cargo configuration. These files are release assets, not vendored into Git.
mkdir -p "$source_root/.cargo"
if [[ ! -d "$source_root/vendor" ]]; then
    cargo vendor --locked --versioned-dirs "$source_root/vendor" >"$stage/vendor-config.toml"
    python - "$stage/vendor-config.toml" "$source_root/.cargo/config.toml" "$source_root/vendor" <<'PY'
from pathlib import Path
import sys
config = Path(sys.argv[1]).read_text()
config = config.replace(sys.argv[3], 'vendor')
Path(sys.argv[2]).write_text(config)
PY
fi

python - "$source_root/vendor" "$binary_root/THIRD-PARTY-NOTICES.txt" <<'PY'
from pathlib import Path
import sys
import tomllib

vendor, output = map(Path, sys.argv[1:])
sections = ['Snap Back third-party notices\n\nThe source archive contains these dependencies in vendor/.\n']
for crate in sorted(vendor.iterdir()):
    manifest = crate / 'Cargo.toml'
    if not manifest.is_file():
        continue
    package = tomllib.loads(manifest.read_text())['package']
    name, version = package['name'], package['version']
    sections.append(f'\n{"=" * 72}\n{name} {version}\nLicense: {package.get("license", "See bundled source")}\nSource: https://crates.io/crates/{name}/{version}\n')
    notices = set()
    for path in crate.iterdir():
        if path.is_file() and path.name.upper().startswith(('LICENSE', 'LICENCE', 'COPYING', 'NOTICE')):
            notices.add(path)
        elif path.is_dir() and path.name.lower() in ('licenses', 'licences'):
            notices.update(item for item in path.rglob('*') if item.is_file())
    if package.get('license-file'):
        path = (crate / package['license-file']).resolve()
        if path.is_relative_to(crate.resolve()) and path.is_file():
            notices.add(path)
    for path in sorted(notices):
        sections.append(f'\n--- {path.relative_to(crate)} ---\n{path.read_text(errors="replace")}\n')
output.write_text(''.join(sections))
PY
install -m644 "$binary_root/THIRD-PARTY-NOTICES.txt" "$source_root/THIRD-PARTY-NOTICES.txt"
for archive_root in "$binary_name" "$source_name"; do
    tar --sort=name --mtime="@$archive_epoch" --owner=0 --group=0 --numeric-owner \
        -C "$stage" -cf - "$archive_root" | gzip -n >"$dist_dir/$archive_root.tar.gz"
done
(
    cd "$dist_dir"
    sha256sum "$binary_name.tar.gz" "$source_name.tar.gz" >SHA256SUMS
)
printf 'Created %s\n' "$dist_dir/$binary_name.tar.gz" "$dist_dir/$source_name.tar.gz" "$dist_dir/SHA256SUMS"
