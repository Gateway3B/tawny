#!/bin/sh
# Never persist the token. Answer only for this one private dependency.
set -eu
[ "${1:-}" = get ] || exit 0
protocol=
host=
path=
while IFS='=' read -r key value; do
    case "$key" in
        protocol) protocol=$value ;;
        host) host=$value ;;
        path) path=$value ;;
    esac
done
[ "$protocol" = https ] && [ "$host" = github.com ] || exit 0
case "$path" in g3techhq/g3-brand|g3techhq/g3-brand.git) ;; *) exit 0 ;; esac
if [ -r /run/secrets/g3_brand_token ]; then
    printf 'username=x-access-token\npassword='
    cat /run/secrets/g3_brand_token
elif [ -n "${G3_BRAND_READ_TOKEN:-}" ]; then
    printf 'username=x-access-token\npassword=%s' "$G3_BRAND_READ_TOKEN"
else
    exit 0
fi
printf '\n\n'
