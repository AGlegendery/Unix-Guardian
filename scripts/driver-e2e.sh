#!/usr/bin/env bash
# End-to-end check of the driver troubleshooter: diagnosis without the
# service, and a fix applied, kept across a restart, and undone through a
# development daemon that works on a scratch tree (--sysroot) and records the
# programs it would run. Nothing touches the real system.
#
#   cargo build --workspace && ./scripts/test-bus.sh ./scripts/driver-e2e.sh
set -uo pipefail
if busctl --user list --activatable 2>/dev/null | grep -q org.unixguardian; then
  echo "FAIL  the test bus can activate an installed Unix Guardian service; run with scripts/test-bus.sh" >&2
  exit 1
fi
B="${1:-$PWD/target/debug}"
W="$(mktemp -d)"
R="$W/root"
fails=0
check() { if eval "$2"; then echo "PASS  $1"; else echo "FAIL  $1"; fails=$((fails+1)); fi; }
until_() { local d=$((SECONDS+$1)); shift; until eval "$@"; do (( SECONDS < d )) || return 1; sleep 0.3; done; }
ug() { "$B/ug" --session "$@"; }

# A scratch machine: Debian, NVIDIA 550 loaded, sleep units off, video
# memory not kept.
mk() { mkdir -p "$(dirname "$R/$1")"; printf '%b' "${2:-}" > "$R/$1"; }
mk etc/os-release 'ID=debian\nPRETTY_NAME="Debian GNU/Linux 13"\n'
mk proc/sys/kernel/osrelease '6.12.0-test\n'
mk proc/cmdline 'ro quiet nvidia-drm.modeset=1\n'
mk proc/modules 'nvidia_drm 1 1 - Live 0x0\nnvidia 1 1 - Live 0x0\n'
mk proc/driver/nvidia/version 'NVRM version: NVIDIA UNIX x86_64 Kernel Module  550.163.01  Tue Apr  8 2025\n'
mk proc/driver/nvidia/params 'PreserveVideoMemoryAllocations: 0\n'
mk usr/lib/x86_64-linux-gnu/libnvidia-ml.so.550.163.01
mk usr/lib/x86_64-linux-gnu/libvulkan.so.1
mk usr/share/vulkan/icd.d/nvidia_icd.json '{}'
mk usr/sbin/update-initramfs
mk etc/modprobe.d/nvidia-blacklists-nouveau.conf 'blacklist nouveau\n'
mk lib/modules/6.12.0-test/modules.dep 'updates/dkms/nvidia.ko.xz:\n'
for u in nvidia-suspend nvidia-resume nvidia-hibernate; do mk "lib/systemd/system/$u.service"; done
cp -r "$R/etc" "$W/etc.before"

"$B/ug-daemon" --session --sysroot "$R" --state-dir "$W/state" --log warn 2>>"$W/daemon.log" & daemon=$!
trap 'kill $daemon 2>/dev/null; rm -rf "$W"' EXIT
until_ 20 "ug driver history >/dev/null 2>&1" || { echo "FAIL  daemon did not start"; cat "$W/daemon.log"; exit 1; }

check "ug diagnose | head ends quietly" "'$B/ug' diagnose 2>'$W/pipe.err' | head -1 >/dev/null; ! grep -q panicked '$W/pipe.err'"
check "ug diagnose works without the service and as JSON" "'$B/ug' diagnose --json | python3 -c 'import json,sys; r=json.load(sys.stdin); assert \"facts\" in r'"
check "the plan shows the file, the units and the rebuild before anything runs" \
  "ug driver plan nvidia-suspend | grep -q '| options nvidia NVreg_PreserveVideoMemoryAllocations=1' && ug driver plan nvidia-suspend | grep -q 'update-initramfs -u -k all' && [ ! -e '$R/etc/modprobe.d/unix-guardian-nvidia-power.conf' ]"
check "a package fix is never run, only shown" "ug driver plan install-headers | grep -q 'Not available here'"
check "answering anything but yes changes nothing" "echo no | ug driver fix nvidia-suspend | grep -q 'Nothing was changed' && [ ! -e '$R/etc/modprobe.d/unix-guardian-nvidia-power.conf' ]"
out="$(ug driver fix nvidia-suspend --yes 2>&1)"
check "the fix runs step by step and finishes" "grep -q ': done' <<<\"\$out\" && grep -q 'Initramfs rebuilt' <<<\"\$out\""
check "our own file is written in modprobe.d" "grep -q 'PreserveVideoMemoryAllocations=1' '$R/etc/modprobe.d/unix-guardian-nvidia-power.conf'"
check "the programs were recorded, not run" "grep -q 'recorded, not run' <<<\"\$out\""
check "the recovery file lists the undo as commands" "grep -q \"sudo rm -f '/etc/modprobe.d/unix-guardian-nvidia-power.conf'\" '$W/state/driver-recovery.txt'"
kill "$daemon"; wait "$daemon" 2>/dev/null
"$B/ug-daemon" --session --sysroot "$R" --state-dir "$W/state" --log warn 2>>"$W/daemon.log" & daemon=$!
until_ 20 "ug driver history >/dev/null 2>&1"
check "the change is remembered after a restart" "ug driver history | grep -q 'done  Keep video memory across sleep'"
check "rollback undoes it to the byte" "ug driver rollback | grep -q undone && diff -r '$W/etc.before' '$R/etc' >/dev/null"
check "and there is nothing left to undo" "! ug driver rollback >/dev/null 2>&1"

# Beyond graphics: Wi-Fi switched off and printing not started are fixed at
# once (no restart), and undone.
mk sys/class/rfkill/rfkill0/type 'wlan\n'
mk sys/class/rfkill/rfkill0/soft '1\n'
mk sys/class/rfkill/rfkill0/hard '0\n'
mk usr/lib/systemd/system/cups.socket
mk usr/lib/systemd/system/cups.service
check "a Wi-Fi switch is flipped without a restart" "ug driver fix rfkill-unblock-wifi --yes | grep -q ': done' && [ \"\$(cat '$R/sys/class/rfkill/rfkill0/soft')\" = 0 ]"
out="$(ug driver fix enable-cups --yes 2>&1)"
check "the printing service is enabled and started" "grep -q 'Enabled cups.socket, cups.service' <<<\"\$out\" && grep -q 'recorded, not run' <<<\"\$out\""
check "both are undone, newest first" "ug driver rollback | grep -q undone && ug driver rollback | grep -q undone && [ \"\$(cat '$R/sys/class/rfkill/rfkill0/soft')\" = 1 ]"
check "the service hands out only firmware lines of the kernel log" "out=\$(busctl --user call org.unixguardian.Daemon1 /org/unixguardian/Daemon1 org.unixguardian.Daemon1.Driver FirmwareLog); [[ \"\$out\" == s* ]]"

if [ "$fails" -gt 0 ]; then echo "--- daemon log"; cat "$W/daemon.log"; fi
echo; [ "$fails" -eq 0 ] && echo "driver-e2e: OK" || { echo "driver-e2e: $fails failed"; exit 1; }
