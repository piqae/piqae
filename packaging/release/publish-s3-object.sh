#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
usage:
  publish-s3-object.sh immutable LOCAL_FILE IMMUTABLE_KEY CONTENT_TYPE
  publish-s3-object.sh promote IMMUTABLE_KEY STABLE_KEY CONTENT_TYPE
  publish-s3-object.sh fetch OPTIONAL_KEY OUTPUT_FILE

Required environment:
  PIQAE_RELEASES_S3_ENDPOINT
  PIQAE_RELEASES_S3_BUCKET
  PIQAE_RELEASES_S3_REGION
  AWS_ACCESS_KEY_ID
  AWS_SECRET_ACCESS_KEY
EOF
  exit 2
}

mode=${1:-}
first=${2:-}
second=${3:-}
content_type=${4:-}

for name in \
  PIQAE_RELEASES_S3_ENDPOINT \
  PIQAE_RELEASES_S3_BUCKET \
  PIQAE_RELEASES_S3_REGION \
  AWS_ACCESS_KEY_ID \
  AWS_SECRET_ACCESS_KEY
do
  if [[ -z "${!name:-}" ]]; then
    echo "required release storage configuration is missing: $name" >&2
    exit 1
  fi
done

if [[ ! "$PIQAE_RELEASES_S3_ENDPOINT" =~ ^https://[A-Za-z0-9][A-Za-z0-9.-]*(:[0-9]+)?/?$ ]]; then
  echo "PIQAE_RELEASES_S3_ENDPOINT must be a safe HTTPS origin" >&2
  exit 2
fi
if [[ ! "$PIQAE_RELEASES_S3_BUCKET" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{1,127}$ ]]; then
  echo "PIQAE_RELEASES_S3_BUCKET is invalid" >&2
  exit 2
fi
if [[ ! "$PIQAE_RELEASES_S3_REGION" =~ ^[A-Za-z0-9][A-Za-z0-9-]{0,62}$ ]]; then
  echo "PIQAE_RELEASES_S3_REGION is invalid" >&2
  exit 2
fi
if ! command -v aws >/dev/null 2>&1 || ! command -v jq >/dev/null 2>&1; then
  echo "aws and jq are required to publish native releases" >&2
  exit 1
fi

aws_args=(
  --endpoint-url "$PIQAE_RELEASES_S3_ENDPOINT"
  --region "$PIQAE_RELEASES_S3_REGION"
  --no-cli-pager
)

validate_immutable_key() {
  local key=$1
  if [[ ! "$key" =~ ^native/releases/[0-9A-Za-z][0-9A-Za-z.+-]{0,63}/[A-Za-z0-9][A-Za-z0-9._/-]{0,220}$ ]] ||
    [[ "$key" == *".."* ]] || [[ "$key" == */ ]]; then
    echo "refusing unsafe immutable release key: $key" >&2
    exit 2
  fi
}

validate_channel_key() {
  local key=$1
  if [[ ! "$key" =~ ^native/(stable|preview)/[A-Za-z0-9][A-Za-z0-9._-]{0,180}$ ]] ||
    [[ "$key" == *".."* ]]; then
    echo "refusing unsafe stable release key: $key" >&2
    exit 2
  fi
}

object_count() {
  local key=$1
  aws "${aws_args[@]}" s3api list-objects-v2 \
    --bucket "$PIQAE_RELEASES_S3_BUCKET" \
    --prefix "$key" \
    --max-keys 1 \
    --output json |
    jq --arg key "$key" '[.Contents[]? | select(.Key == $key)] | length'
}

verify_object() {
  local key=$1
  local expected_sha=$2
  local expected_size=$3
  local head
  head=$(aws "${aws_args[@]}" s3api head-object \
    --bucket "$PIQAE_RELEASES_S3_BUCKET" \
    --key "$key" \
    --output json)
  jq -e \
    --arg sha "$expected_sha" \
    --argjson size "$expected_size" \
    '.ContentLength == $size and .Metadata.sha256 == $sha' \
    <<<"$head" >/dev/null
}

file_size() {
  local file=$1
  local bytes
  bytes=$(LC_ALL=C wc -c <"$file")
  bytes=${bytes//[[:space:]]/}
  if [[ ! "$bytes" =~ ^[0-9]+$ ]]; then
    echo "could not determine release object size: $file" >&2
    exit 1
  fi
  printf '%s\n' "$bytes"
}

case "$mode" in
immutable)
  local_file=$first
  immutable_key=$second
  [[ -n "$local_file" && -n "$immutable_key" && -n "$content_type" ]] || usage
  [[ -f "$local_file" && -s "$local_file" ]] || {
    echo "release input is missing or empty: $local_file" >&2
    exit 1
  }
  validate_immutable_key "$immutable_key"
  sha=$(shasum -a 256 "$local_file" | awk '{print $1}')
  size=$(file_size "$local_file")
  if [[ "$(object_count "$immutable_key")" != 0 ]]; then
    if verify_object "$immutable_key" "$sha" "$size"; then
      # A retry may reuse an immutable object only when its bytes are identical.
      exit 0
    fi
    echo "refusing to overwrite immutable release object with different bytes: $immutable_key" >&2
    exit 1
  fi
  aws "${aws_args[@]}" s3api put-object \
    --bucket "$PIQAE_RELEASES_S3_BUCKET" \
    --key "$immutable_key" \
    --body "$local_file" \
    --content-type "$content_type" \
    --metadata "sha256=$sha" \
    --output json >/dev/null
  verify_object "$immutable_key" "$sha" "$size"
  ;;
promote)
  immutable_key=$first
  stable_key=$second
  [[ -n "$immutable_key" && -n "$stable_key" && -n "$content_type" ]] || usage
  validate_immutable_key "$immutable_key"
  validate_channel_key "$stable_key"
  scratch=$(mktemp)
  trap 'rm -f -- "$scratch"' EXIT
  head=$(aws "${aws_args[@]}" s3api head-object \
    --bucket "$PIQAE_RELEASES_S3_BUCKET" \
    --key "$immutable_key" \
    --output json)
  expected_sha=$(jq -er '.Metadata.sha256' <<<"$head")
  expected_size=$(jq -er '.ContentLength' <<<"$head")
  aws "${aws_args[@]}" s3api get-object \
    --bucket "$PIQAE_RELEASES_S3_BUCKET" \
    --key "$immutable_key" \
    "$scratch" \
    --output json >/dev/null
  actual_sha=$(shasum -a 256 "$scratch" | awk '{print $1}')
  actual_size=$(file_size "$scratch")
  if [[ "$actual_sha" != "$expected_sha" || "$actual_size" != "$expected_size" ]]; then
    echo "immutable release object failed download verification: $immutable_key" >&2
    exit 1
  fi
  aws "${aws_args[@]}" s3api put-object \
    --bucket "$PIQAE_RELEASES_S3_BUCKET" \
    --key "$stable_key" \
    --body "$scratch" \
    --content-type "$content_type" \
    --metadata "sha256=$expected_sha" \
    --output json >/dev/null
  verify_object "$stable_key" "$expected_sha" "$expected_size"
  ;;
fetch)
  key=$first
  output=$second
  [[ -n "$key" && -n "$output" && -z "$content_type" ]] || usage
  validate_channel_key "$key"
  if [[ "$(object_count "$key")" == 0 ]]; then
    exit 3
  fi
  aws "${aws_args[@]}" s3api get-object \
    --bucket "$PIQAE_RELEASES_S3_BUCKET" \
    --key "$key" \
    "$output" \
    --output json >/dev/null
  ;;
*)
  usage
  ;;
esac
