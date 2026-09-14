#!/usr/bin/env bash
set -euo pipefail

die() {
	printf 'Error: %s\n' "$*" >&2
	exit 1
}

command -v cargo >/dev/null 2>&1 || die "cargo not found"

BRANCH=$(git rev-parse --abbrev-ref HEAD)
[[ "$BRANCH" == "master" ]] || die "must be on 'master' branch, currently on '$BRANCH'"

git diff --quiet || die "working tree is dirty — commit or stash changes first"
git diff --cached --quiet || die "staged changes present"

VERSION=$(cargo pkgid | cut -d'#' -f2)

git rev-parse "v$VERSION" >/dev/null 2>&1 && die "tag v$VERSION already exists — bump Cargo.toml first"

git add Cargo.toml Cargo.lock
if git diff --cached --quiet; then
	echo "Nothing to commit — Cargo.toml already at $VERSION, creating tag only."
else
	git commit -m "chore: bump version to $VERSION"
fi
git tag -m "v$VERSION" "v$VERSION"
echo "Done: v$VERSION tagged locally."
echo ""
echo "Next steps:"
echo "  git push origin master"
echo "  git push origin v$VERSION"
echo ""
echo "CI will build and publish the release."
