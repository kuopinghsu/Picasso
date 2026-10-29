#!/usr/bin/env bash
# Installs packages with dnf in the AlmaLinux containers of CI
# (.github/workflows/ci.yml). The mirror list now and then hands out a mirror
# serving empty metadata, which fails every retry against it, so the
# repositories are read from repo.almalinux.org itself; a failed attempt is
# tried again after a pause.
#   bash scripts/dnf-install.sh PACKAGE...
set -euo pipefail

for repo in /etc/yum.repos.d/almalinux*.repo; do
	if grep -q '^# *baseurl=' "$repo"; then
		sed -i 's|^mirrorlist=|#mirrorlist=|; s|^# *baseurl=|baseurl=|' "$repo"
	fi
done

for attempt in 1 2 3; do
	dnf install -y --setopt=install_weak_deps=False "$@" && exit 0
	echo "::warning::dnf install failed (attempt $attempt), trying again"
	dnf clean all
	sleep $((attempt * 20))
done
exit 1
