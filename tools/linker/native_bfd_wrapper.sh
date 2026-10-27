#!/usr/bin/env bash
# Sourceable helper for tools/linker/setup_oracles.sh.
#
# The cached GNU ld 2.47 binary is built with upstream configure defaults: it
# uses old dtags and has no Debian multiarch library directories in its search
# list. The native oracle contract is supplied by a self-contained wrapper,
# not by a stamp pretending those settings were compiled into the binary.
#
# Usage after sourcing this file:
#   write_native_bfd_wrapper OUTPUT REAL_LD NATIVE_LIB_PATH
write_native_bfd_wrapper() {
    local output=$1 real_ld=$2 native_lib_path=$3
    {
        printf '#!/usr/bin/env bash\nset -euo pipefail\n'
        printf 'bfd=%q\n' "$real_ld"
        printf 'native_lib_path=%q\n' "$native_lib_path"
        cat <<'WRAPPER'
args=("$@")
sysroot=/
nostdlib=0
for ((index = 0; index < ${#args[@]}; index++)); do
    arg=${args[index]}
    case "$arg" in
        -nostdlib|--nostdlib) nostdlib=1 ;;
        --sysroot=*) sysroot=${arg#--sysroot=} ;;
        --sysroot)
            if ((index + 1 < ${#args[@]})); then
                index=$((index + 1))
                sysroot=${args[index]}
            else
                sysroot=
            fi
            ;;
    esac
done

# These are defaults: options supplied by the caller follow them and can
# override either setting. In particular, --disable-new-dtags remains usable.
link_args=(--sysroot=/ --enable-new-dtags "${args[@]}")
if ((nostdlib == 0)); then
    search_root=${sysroot%/}
    if [[ $sysroot == / ]]; then
        search_root=
    fi
    IFS=: read -r -a search_dirs <<< "$native_lib_path"
    for dir in "${search_dirs[@]}"; do
        [[ -n $dir ]] || continue
        link_args+=("-L${search_root}${dir}")
    done
fi
exec "$bfd" "${link_args[@]}"
WRAPPER
    } > "$output"
    chmod +x "$output"
}
