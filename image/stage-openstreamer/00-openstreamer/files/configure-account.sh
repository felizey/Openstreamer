#!/bin/bash
# Runs only inside the image rootfs. Cached Raspberry Pi OS stages may have
# been built with another first-user name; keep their UID and group membership.
set -euo pipefail

EXPECTED=$1
PASSWORD=$2
[[ "$EXPECTED" =~ ^[a-z_][a-z0-9_-]*$ ]] || {
    echo "Invalid image account name" >&2
    exit 1
}
PREVIOUS=$(getent passwd 1000 | cut -d: -f1)
[[ "$PREVIOUS" =~ ^[a-z_][a-z0-9_-]*$ ]] || {
    echo "The image must have exactly one first user with UID 1000" >&2
    exit 1
}

if [ "$PREVIOUS" != "$EXPECTED" ]; then
    PREVIOUS_GROUP=$(id -gn "$PREVIOUS")
    usermod --login "$EXPECTED" --home "/home/$EXPECTED" --move-home "$PREVIOUS"
    if [ "$PREVIOUS_GROUP" = "$PREVIOUS" ]; then
        groupmod --new-name "$EXPECTED" "$PREVIOUS_GROUP"
    fi

    # These are generated image files, including first-boot/cloud-init inputs.
    # Match complete account names so a short name cannot alter distro names.
    rewrite() {
        sed -E ":again;s/(^|[^[:alnum:]_-])$PREVIOUS([^[:alnum:]_-]|$)/\1$EXPECTED\2/g;tagain"
    }
    SCAN=(/etc /boot/firmware "/home/$EXPECTED")
    for DIR in /var/lib/cloud /var/lib/AccountsService /var/spool; do
        [ ! -d "$DIR" ] || SCAN+=("$DIR")
    done
    while IFS= read -r -d '' FILE; do
        if grep -Iq . "$FILE"; then
            BEFORE=$(cat "$FILE")
            AFTER=$(printf '%s' "$BEFORE" | rewrite)
            if [ "$BEFORE" != "$AFTER" ]; then
                # Preserve owner, permissions and the file's final newline.
                sed -i -E ":again;s/(^|[^[:alnum:]_-])$PREVIOUS([^[:alnum:]_-]|$)/\1$EXPECTED\2/g;tagain" "$FILE"
            fi
        fi
    done < <(find "${SCAN[@]}" -type f -print0)
    while IFS= read -r -d '' LINK; do
        BEFORE=$(readlink "$LINK")
        AFTER=$(printf '%s' "$BEFORE" | rewrite)
        [ "$BEFORE" = "$AFTER" ] || ln -sfn "$AFTER" "$LINK"
    done < <(find "${SCAN[@]}" -type l -print0)
    while IFS= read -r -d '' FILE; do
        BEFORE=${FILE##*/}
        # Generated rule filenames use '_' and '-' around the account name.
        AFTER=$(printf '%s' "$BEFORE" | sed -E ":again;s/(^|[^[:alnum:]])$PREVIOUS([^[:alnum:]]|$)/\1$EXPECTED\2/g;tagain")
        [ "$BEFORE" = "$AFTER" ] || mv -- "$FILE" "${FILE%/*}/$AFTER"
    done < <(find "${SCAN[@]}" -depth -name "*$PREVIOUS*" -print0)

    # Account tools leave backups containing the old login. The image has no
    # user activity yet, so first-boot state should also be regenerated.
    rm -f /etc/passwd- /etc/shadow- /etc/group- /etc/gshadow-
    rm -rf /var/lib/cloud/instances /var/lib/cloud/instance /var/lib/cloud/sem /var/lib/cloud/data
    if grep -rIlw -- "$PREVIOUS" "${SCAN[@]}"; then
        echo "A cached first-user name remains in the image configuration" >&2
        exit 1
    fi
fi

printf '%s:%s\n' "$EXPECTED" "$PASSWORD" | chpasswd
usermod --shell /bin/bash "$EXPECTED"
mkdir -p /etc/cloud/cloud.cfg.d
cat > /etc/cloud/cloud.cfg.d/99-openstreamer-user.cfg <<EOF
system_info:
  default_user:
    name: $EXPECTED
    gecos: Openstreamer
    homedir: /home/$EXPECTED
    lock_passwd: false
EOF
id "$EXPECTED"
[ "$(getent passwd "$EXPECTED" | cut -d: -f3)" = 1000 ]
[ "$(getent passwd "$EXPECTED" | cut -d: -f6)" = "/home/$EXPECTED" ]
visudo --check
