# Mount-table helpers shared by the destructive-path guards
# (prepare_kernel_tree.sh and ci_ubuntu_chroot.sh). Source this file; it only
# defines functions.
#
# The kernel escapes space, tab, newline and backslash in the mount-point field
# of /proc/self/mountinfo as \040 \011 \012 \134 (fs/proc_namespace.c, mangle()).
# A raw string comparison misses a mount whose path contains a space, and a
# deletion guard that misses a mount is the failure this file exists to stop.
# The escapes are therefore decoded before any comparison.

# list_mounts_under DIR
#   Prints each decoded mount point equal to DIR or lying below it. DIR must be
#   an absolute, normalised path. "/" lists every mount.
#   Exit status 1 when the table is unreadable (callers must treat that as
#   "a mount may exist").
#   LCCC_MOUNTINFO overrides the table path; tests use it with a fixture.
list_mounts_under() {
    local info=${LCCC_MOUNTINFO:-/proc/self/mountinfo}
    [[ -r $info ]] || return 1
    # The directory travels in the environment: `awk -v` would interpret
    # backslash escapes in it, so a literal `\040` in DIR would become a space.
    LCCC_MI_DIR=$1 awk '
        function oct(s) { return substr(s, 1, 1) * 64 + substr(s, 2, 1) * 8 + substr(s, 3, 1) }
        # Decodes \NNN octal escapes in one left-to-right pass, so an escaped
        # backslash (\134) followed by digits is not decoded twice.
        function unescape(s,    out, i, o) {
            out = ""
            while ((i = index(s, "\\")) > 0) {
                out = out substr(s, 1, i - 1)
                o = substr(s, i + 1, 3)
                if (o ~ /^[0-7][0-7][0-7]$/) {
                    out = out sprintf("%c", oct(o))
                    s = substr(s, i + 4)
                } else {
                    out = out "\\"
                    s = substr(s, i + 1)
                }
            }
            return out s
        }
        BEGIN { d = ENVIRON["LCCC_MI_DIR"]; prefix = (d == "/") ? "" : d }
        {
            m = unescape($5)
            if (m == d || index(m, prefix "/") == 1) print m
        }' "$info"
}

# has_mount_under DIR
#   Exit 0 when a mount point is DIR or lies below it, or when the mount table
#   cannot be read (fail closed: deleting through an unseen mount is the risk).
has_mount_under() {
    local found
    found=$(list_mounts_under "$1") || return 0
    [[ -n $found ]]
}
