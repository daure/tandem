# Sourced by installers after validating the replacement, before publishing it.
_upgrade_services=''
upgrade_cleanup() { :; }

upgrade_matches() {
    [ -O "/proc/$1" ] || return 1
    _upgrade_exe=$(readlink "/proc/$1/exe" 2>/dev/null) || return 1
    _upgrade_exe=${_upgrade_exe%" (deleted)"}
    if [ -n "$_upgrade_root" ]; then
        case "$_upgrade_exe" in "$_upgrade_allowed"/*) return 1 ;; esac
        case "$_upgrade_exe" in "$_upgrade_root"/versions/*) return 0 ;; *) return 1 ;; esac
    fi
    [ "$_upgrade_exe" = "$_upgrade_binary" ] || return 1
    ! cmp -s "$_upgrade_replacement" "/proc/$1/exe"
}

upgrade_scan() {
    for _upgrade_proc in /proc/[0-9]*; do
        _upgrade_pid=${_upgrade_proc##*/}
        if upgrade_matches "$_upgrade_pid"; then printf '%s\n' "$_upgrade_pid"; fi
    done
}

upgrade_service() {
    _upgrade_unit=''
    [ -r "/proc/$1/cgroup" ] || return 0
    if cmp -s "/proc/$1/cgroup" "/proc/$$/cgroup"; then return 0; fi
    while IFS=: read -r _upgrade_hierarchy _upgrade_controllers _upgrade_group; do
        case "$_upgrade_group" in
            */user@*.service/*)
                _upgrade_tail=${_upgrade_group#*/user@}
                _upgrade_tail=${_upgrade_tail#*/}
                while [ -n "$_upgrade_tail" ]; do
                    _upgrade_part=${_upgrade_tail%%/*}
                    case "$_upgrade_part" in *.service) _upgrade_unit=$_upgrade_part ;; esac
                    case "$_upgrade_tail" in */*) _upgrade_tail=${_upgrade_tail#*/} ;; *) break ;; esac
                done ;;
        esac
    done < "/proc/$1/cgroup"
    [ -n "$_upgrade_unit" ] || return 0
    _upgrade_main=$(timeout 30 systemctl --user show "$_upgrade_unit" --property=MainPID --value) || return 1
    case "$_upgrade_main" in ''|0|*[!0-9]*) return 0 ;; esac
    # Client-hosting services can own unrelated apps; restart only this installation's service.
    if upgrade_matches "$_upgrade_main"; then printf '%s\n' "$_upgrade_unit"; fi
}

upgrade_resume() {
    _upgrade_resume_status=0
    for _upgrade_service in $_upgrade_services; do
        if ! timeout 30 systemctl --user start "$_upgrade_service" ||
           ! timeout 30 systemctl --user is-active --quiet "$_upgrade_service"; then
            printf 'Could not restart %s; inspect systemctl --user status %s.\n' "$_upgrade_service" "$_upgrade_service" >&2
            _upgrade_resume_status=1
        fi
    done
    _upgrade_services=''
    return "$_upgrade_resume_status"
}

upgrade_on_exit() {
    _upgrade_status=$1
    trap - 0 HUP INT TERM
    upgrade_resume || _upgrade_status=1
    upgrade_cleanup
    exit "$_upgrade_status"
}

upgrade_prepare() {
    _upgrade_directory=$(CDPATH='' cd -- "${1%/*}" && pwd -P) || return 1
    _upgrade_binary=$_upgrade_directory/${1##*/}
    _upgrade_replacement=$2
    _upgrade_root=${3:-}
    _upgrade_allowed=${4:-}
    [ -d /proc/self ] || { printf 'Cannot inspect running processes; upgrade aborted.\n' >&2; return 1; }
    if [ ! -x "$_upgrade_replacement" ] || ! "$_upgrade_replacement" --version >/dev/null; then
        printf 'Replacement executable failed its version check; upgrade aborted.\n' >&2
        return 1
    fi
    for _upgrade_command in readlink cmp flock timeout; do
        command -v "$_upgrade_command" >/dev/null 2>&1 || { printf 'Required installer command: %s\n' "$_upgrade_command" >&2; return 1; }
    done
    exec 9>"${_upgrade_root:-${_upgrade_binary%/*}}/.${_upgrade_binary##*/}-upgrade.lock" || return 1
    flock -w 30 9 || return 1
    _upgrade_pids=$(upgrade_scan)
    [ -n "$_upgrade_pids" ] || return 0
    set -- $_upgrade_pids
    printf 'Found %s outdated processes from %s:\n' "$#" "$_upgrade_binary" >&2
    for _upgrade_pid in $_upgrade_pids; do
        _upgrade_unit=$(upgrade_service "$_upgrade_pid") || { printf 'Cannot inspect a process service; upgrade aborted.\n' >&2; _upgrade_services=''; return 1; }
        _upgrade_name=process
        IFS= read -r _upgrade_name < "/proc/$_upgrade_pid/comm" 2>/dev/null || :
        printf '  %s (PID %s)%s\n' "$_upgrade_name" "$_upgrade_pid" "${_upgrade_unit:+ (service $_upgrade_unit)}" >&2
        if [ -n "$_upgrade_unit" ]; then
            case " $_upgrade_services " in *" $_upgrade_unit "*) ;; *) _upgrade_services="$_upgrade_services $_upgrade_unit" ;; esac
        fi
    done
    printf 'Stop these processes and restart their services for the upgrade? Active work and calls may be interrupted; unresponsive processes will be killed. [y/N] ' >&2
    _upgrade_reply=''
    if [ -t 0 ]; then
        IFS= read -r _upgrade_reply || :
    elif (exec 3<>/dev/tty) 2>/dev/null; then
        IFS= read -r _upgrade_reply </dev/tty || :
    fi
    case "$_upgrade_reply" in
        y|Y|yes|YES) ;;
        *) _upgrade_services=''; printf '\nUpgrade cancelled; stop the listed processes or rerun from a terminal to approve.\n' >&2; return 1 ;;
    esac
    trap 'upgrade_on_exit $?' 0
    trap 'exit 129' HUP
    trap 'exit 130' INT
    trap 'exit 143' TERM
    for _upgrade_service in $_upgrade_services; do
        timeout 30 systemctl --user stop "$_upgrade_service" || return 1
    done
    for _upgrade_signal in TERM KILL; do
        _upgrade_pids=$(upgrade_scan)
        for _upgrade_pid in $_upgrade_pids; do
            if upgrade_matches "$_upgrade_pid"; then kill -"$_upgrade_signal" "$_upgrade_pid" 2>/dev/null || :; fi
        done
        _upgrade_wait=0
        while [ -n "$(upgrade_scan)" ] && [ "$_upgrade_wait" -lt 5 ]; do
            sleep 1
            _upgrade_wait=$((_upgrade_wait + 1))
        done
        [ -n "$(upgrade_scan)" ] || return 0
    done
    printf 'Processes are still running; upgrade aborted.\n' >&2
    return 1
}

upgrade_finish() {
    _upgrade_replacement=$_upgrade_binary
    if [ -n "$(upgrade_scan)" ]; then
        printf 'Outdated processes are running; installation is incomplete.\n' >&2
        return 1
    fi
    upgrade_resume || return 1
    if [ -n "$(upgrade_scan)" ]; then
        printf 'A service started an outdated executable; installation is incomplete.\n' >&2
        return 1
    fi
    printf 'No outdated processes remain. Reopen stopped apps and reconnect client-managed MCP servers.\n' >&2
}
