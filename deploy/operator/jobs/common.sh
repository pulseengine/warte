# Shared by the job scripts. Sourced, not run.
# WARTE_URL      warte's base URL, e.g. https://warte.example.internal/warte
# WARTE_CA_CERT  optional: PEM CA for a proxy signed by a local CA
# The job's token is read from a file, never from the command line or the
# environment: $CREDENTIALS_DIRECTORY/warte-token under systemd
# (LoadCredential=), else $WARTE_TOKEN_FILE. Mint it on the ground station:
#   warte token mint <job-name> --valid-hours 720 --right <right>
set -eu
: "${WARTE_URL:?set WARTE_URL}"
token_file="${CREDENTIALS_DIRECTORY:+$CREDENTIALS_DIRECTORY/warte-token}"
token_file="${token_file:-${WARTE_TOKEN_FILE:?set WARTE_TOKEN_FILE or use LoadCredential=warte-token:...}}"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
# the Authorization header goes to curl from a file, so it is not in ps
umask 077
printf 'Authorization: Bearer %s\n' "$(cat "$token_file")" > "$work/auth"

# api GET|POST <path> [<file with the JSON body>]
api() {
	if [ -n "${WARTE_CA_CERT:-}" ]; then ca="--cacert $WARTE_CA_CERT"; else ca=""; fi
	if [ "$1" = POST ]; then
		# shellcheck disable=SC2086
		curl -fsS $ca -H @"$work/auth" -H 'Content-Type: application/json' \
			--data-binary @"$3" "$WARTE_URL$2"
	else
		# shellcheck disable=SC2086
		curl -fsS $ca -H @"$work/auth" "$WARTE_URL$2"
	fi
}
