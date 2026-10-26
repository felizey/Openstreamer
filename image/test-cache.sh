#!/usr/bin/env bash
# Shared content-addressed cache helpers. Sourcing this file has no side effects.

test_cache_take_lock() {
    local checkout=$1
    command -v flock >/dev/null || {
        echo 'This builder requires flock (the util-linux package on Ubuntu).' >&2
        return 1
    }
    mkdir -p -- "$checkout" || return
    # Keep the descriptor open throughout staging, Docker execution and copyout.
    # Never unlink the lock file: another process may already have opened it.
    exec 9> "$checkout/.openstreamer-test-build.lock" || return
    flock -n 9 || {
        echo 'Another Openstreamer test build is using this checkout. Wait for it to finish.' >&2
        return 1
    }
}

test_cache_digest() {
    sha256sum | cut -d ' ' -f 1
}

test_cache_key() {
    printf '%s\0' "$@" | test_cache_digest
}

test_cache_tree_digest() (
    cd -- "$1" || return
    # File paths and bytes matter; checkout timestamps and uid/gid do not.
    {
        find . -type f -print0 | LC_ALL=C sort -z | xargs -0 -r sha256sum --zero
        while IFS= read -r -d '' link; do
            printf 'link\0%s\0%s\0' "$link" "$(readlink -- "$link")"
        done < <(find . -type l -print0 | LC_ALL=C sort -z)
    } | test_cache_digest
)

test_cache_stage_matches() {
    local stage_dir=$1 expected=$2
    [ -f "$stage_dir/rootfs/etc/os-release" ] &&
        [ -f "$stage_dir/rootfs/var/lib/dpkg/status" ] &&
        [ -f "$stage_dir/.openstreamer-test-cache-key" ] &&
        [ "$(cat "$stage_dir/.openstreamer-test-cache-key")" = "$expected" ]
}

test_cache_image_matches() {
    local image=$1 hash_file=$2 expected actual_name
    [ -f "$image" ] && [ -f "$hash_file" ] || return 1
    expected=$(cat "$hash_file") || return
    actual_name=$(basename -- "$image")
    # Reject malformed metadata and references to a different image. Validate
    # this exact artifact rather than arbitrary paths from a checksum file.
    [[ "$expected" =~ ^[0-9a-f]{64}\ \ "$actual_name"$ ]] || return 1
    (cd -- "$(dirname -- "$image")" && printf '%s\n' "$expected" | sha256sum --status --check -)
}

test_cache_write_key() {
    local stage_dir=$1 key=$2 temporary
    temporary=$(mktemp "$stage_dir/.openstreamer-test-cache-key.XXXXXX") || return
    printf '%s\n' "$key" > "$temporary" &&
        mv -f -- "$temporary" "$stage_dir/.openstreamer-test-cache-key"
}

test_cache_restore_pristine() {
    local source=$1 destination=$2 work_dir=$3 resolved_source resolved_destination resolved_work
    resolved_source=$(realpath -e -- "$source") || return
    resolved_destination=$(realpath -m -- "$destination") || return
    resolved_work=$(realpath -e -- "$work_dir") || return
    # Every destructive sync is restricted to these two builder-owned stages.
    case "$resolved_destination" in
        "$resolved_work/stage-openstreamer/rootfs"|"$resolved_work/stage-openstreamer-runtime/rootfs") ;;
        *) echo "Refusing cache restore outside the isolated application/runtime rootfs: $destination" >&2; return 1 ;;
    esac
    case "$resolved_source" in
        "$resolved_work/stage2/rootfs"|"$resolved_work/stage-openstreamer-runtime/rootfs") ;;
        *) echo "Refusing cache restore from an unrelated rootfs: $source" >&2; return 1 ;;
    esac
    [ "$resolved_source" != "$resolved_destination" ] && [ ! -L "$destination" ] || return 1
    [ -f "$resolved_source/etc/os-release" ] && [ -f "$resolved_source/var/lib/dpkg/status" ] || return 1
    # pi-gen normally unmounts stages first. Fail closed if a mount survived;
    # --delete must never traverse a live chroot bind mount.
    if findmnt -rn -o TARGET | awk -v destination="$resolved_destination" '
        $0 == destination || index($0, destination "/") == 1 { found = 1 }
        END { exit !found }
    '; then
        echo "Refusing cache restore while the target rootfs contains a mount: $destination" >&2
        return 1
    fi
    mkdir -p -- "$destination" || return
    # The previous success marker stops being valid before the first write.
    # An interrupted A -> B build must never let a later request for A treat
    # the partially modified B rootfs as a complete A cache hit.
    rm -f -- "$(dirname -- "$destination")/.openstreamer-test-cache-key" || return
    # A checksum comparison also restores files modified without a size/mtime
    # change. --delete removes old app assets, users, service links and settings.
    # Unchanged OS files retain their existing blocks instead of being recopied.
    rsync -aHAXx --numeric-ids --delete --checksum \
        --exclude /var/cache/apt/archives/ "$resolved_source/" "$destination/"
}
