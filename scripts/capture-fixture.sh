#!/usr/bin/env bash
# Records this machine's hardware layout as a replayable fixture.
#
# Read-only. Needs no root — files only root can read are skipped and listed.
# Run it on a machine whose hardware Unix Guardian should support, then send
# the resulting .tar.gz back; the parsers get tested against it everywhere.
#
#   ./scripts/capture-fixture.sh              # -> fixture-<host>-<date>.tar.gz
#   ./scripts/capture-fixture.sh my-rx6700    # -> fixture-my-rx6700.tar.gz
#
# Replay with:  ug-daemon --session --sysroot <extracted dir>
#
# What is NOT captured: no serial numbers, MAC addresses, user files or
# process list. The default gateway address is zeroed out of the route table,
# and filesystem UUIDs are zeroed out of the kernel command line (its other
# parameters are kept — the driver checks read them).
set -euo pipefail

name="${1:-$(hostname -s)-$(date +%Y%m%d)}"
out="$(mktemp -d)/fixture-$name"
mkdir -p "$out"
skipped=()

# copy_file SRC — copies one regular file (following sysfs symlinks for the
# *path*, but storing it under the logical path the reader uses).
copy_file() {
  local src="$1" dst="$out$1"
  [ -f "$src" ] || return 0
  mkdir -p "$(dirname "$dst")"
  if ! cat "$src" > "$dst" 2>/dev/null; then
    rm -f "$dst"; skipped+=("$src")
  fi
  return 0
}

copy_matching() {  # copy_matching DIR REGEX
  local dir="$1" re="$2" f
  [ -d "$dir" ] || return 0
  for f in "$dir"/*; do
    if [[ -f "$f" && "$(basename "$f")" =~ $re ]]; then copy_file "$f"; fi
  done
  return 0  # under set -e, a false test on the last iteration must not abort
}

echo "capturing into $out"

# --- hwmon chips: names, labels and readings; never pwm_enable writes.
for h in /sys/class/hwmon/hwmon*; do
  copy_matching "$h" '^(name|(temp|fan|power|in|curr|pwm)[0-9]+(_[a-z_]+)?)$'
done

# --- GPUs: identity, amdgpu telemetry and overdrive tables, hwmon, runtime PM.
for c in /sys/class/drm/card[0-9]*; do
  [[ "$(basename "$c")" =~ ^card[0-9]+$ ]] || continue
  d="$c/device"
  copy_matching "$d" '^(vendor|device|subsystem_vendor|subsystem_device|boot_vga|uevent|gpu_busy_percent|mem_busy_percent|mem_info_vram_(total|used)|pp_dpm_(sclk|mclk|fclk|socclk)|pp_od_clk_voltage|pp_power_profile_mode|power_dpm_force_performance_level|current_link_(speed|width))$'
  if [ -d "$d/gpu_od" ]; then
    while read -r f; do copy_file "$f"; done < <(find "$d/gpu_od" -type f 2>/dev/null)
  fi
  for h in "$d"/hwmon/hwmon*; do
    copy_matching "$h" '^(name|(temp|fan|power|in|pwm)[0-9]+(_[a-z_]+)?|power1_cap(_min|_max|_default)?)$'
  done
  # The bound driver is a symlink; keep it as one (dangling is fine — readers
  # only look at the link's final component).
  if drv=$(readlink "$d/driver" 2>/dev/null); then
    mkdir -p "$out$d"; ln -sfn "$drv" "$out$d/driver"
  fi
  pci=$(sed -n 's/^PCI_SLOT_NAME=//p' "$d/uevent" 2>/dev/null || true)
  if [ -n "$pci" ]; then copy_matching "/sys/bus/pci/devices/$pci/power" '^(runtime_status|control)$'; fi
done

# --- CPU: frequency policy, topology and caches.
copy_file /sys/devices/system/cpu/online
copy_file /sys/devices/cpu_core/cpus   # Intel hybrid: P-cores
copy_file /sys/devices/cpu_atom/cpus   # Intel hybrid: E-cores
copy_file /sys/devices/system/cpu/smt/active
for cpu in /sys/devices/system/cpu/cpu[0-9]*; do
  copy_matching "$cpu/cpufreq" '^(scaling_(governor|driver|cur_freq|min_freq|max_freq|available_governors)|energy_performance_(preference|available_preferences)|cpuinfo_(min|max)_freq)$'
  copy_matching "$cpu/topology" '^(core_cpus_list|thread_siblings_list|physical_package_id|core_id|die_id)$'
  for i in "$cpu"/cache/index*; do copy_matching "$i" '^(level|type|size|id|shared_cpu_list)$'; done
done

# --- Power, memory, network, distro.
for p in /sys/class/powercap/intel-rapl:*; do copy_matching "$p" '^(name|energy_uj|max_energy_range_uj)$'; done
for z in /sys/block/zram*; do copy_matching "$z" '^(mm_stat|disksize)$'; done
copy_file /proc/stat
copy_file /proc/meminfo
copy_file /proc/cpuinfo
copy_file /proc/version
copy_file /proc/sys/kernel/osrelease
if [ -r /proc/cmdline ]; then
  mkdir -p "$out/proc"
  sed -E 's/[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}/00000000-0000-0000-0000-000000000000/g' \
    /proc/cmdline > "$out/proc/cmdline"
fi
copy_file /etc/os-release
if [ -f /proc/net/route ]; then  # keep interface and metric, drop the gateway
  mkdir -p "$out/proc/net"
  awk 'NR==1{print;next}{$3="00000000";print}' OFS='\t' /proc/net/route > "$out/proc/net/route"
fi
iface=$(awk 'NR>1 && $2=="00000000"{print $1; exit}' /proc/net/route 2>/dev/null || true)
if [ -n "$iface" ]; then copy_matching "/sys/class/net/$iface/statistics" '^(rx_bytes|tx_bytes)$'; fi

# A short human summary, so a fixture can be identified without extracting it.
{
  echo "captured: $(date -Iseconds)"
  echo "kernel:   $(uname -r)"
  . /etc/os-release 2>/dev/null && echo "distro:   ${PRETTY_NAME:-unknown}"
  echo "cpu:      $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2- | xargs)"
  for c in /sys/class/drm/card[0-9]*; do
    [[ "$(basename "$c")" =~ ^card[0-9]+$ ]] || continue
    echo "gpu:      $(basename "$c") $(cat "$c/device/vendor" 2>/dev/null):$(cat "$c/device/device" 2>/dev/null) driver=$(basename "$(readlink "$c/device/driver" 2>/dev/null)" 2>/dev/null)"
  done
  if command -v nvidia-smi >/dev/null 2>&1; then
    echo; echo "# nvidia-smi (NVIDIA has no sysfs telemetry; this is the reference)"
    nvidia-smi --query-gpu=name,driver_version,temperature.gpu,power.draw,power.limit,clocks.gr,clocks.mem,fan.speed --format=csv 2>/dev/null || true
  fi
  if [ "${#skipped[@]}" -gt 0 ]; then
    echo; echo "# not readable without root (skipped):"
    printf '%s\n' "${skipped[@]}"
  fi
} > "$out/FIXTURE.txt"

tarball="$PWD/fixture-$name.tar.gz"
tar -C "$(dirname "$out")" -czf "$tarball" "$(basename "$out")"
rm -rf "$(dirname "$out")"
echo
echo "done: $tarball ($(du -h "$tarball" | cut -f1))"
echo "${#skipped[@]} root-only files skipped; see FIXTURE.txt inside."
