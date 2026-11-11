#!/usr/bin/env bash
# The steps of the Fearlezz action (action.yml), one subcommand each. They run on GitHub
# Actions and read what GitHub hands every step: GITHUB_EVENT_PATH, GITHUB_REPOSITORY,
# RUNNER_TEMP, GITHUB_OUTPUT and the rest. GH_TOKEN is the token that comments, sets statuses
# and pushes the pictures.
#
#   task          which job this event asks for: prove, approve, clean, squash or none
#   install       install Fearlezz ($VERSION: latest, vX.Y.Z, source or installed)
#   needs-ffmpeg  whether to install ffmpeg ($FFMPEG: auto, true or false)
#   ffmpeg        install a pinned static ffmpeg into $FFMPEG_DIR
#   prove         prove this commit, with the changed pictures in $RUNNER_TEMP
#   publish       push the changed pictures to the pictures branch
#   comment       post or replace the status comment on the pull request
#   status        set the fearlezz/pictures status of the pull request's head
#   approve       answer a `/fearlezz approve <words>` comment
#   clean         remove a closed pull request's pictures and squash the branch
#   squash        remove the pictures of every closed pull request and squash the branch
set -euo pipefail

branch="${PICTURES_BRANCH:-fearlezz-pictures}"
repo="repos/$GITHUB_REPOSITORY"
event="${GITHUB_EVENT_PATH:-/dev/null}"
comment_file="$RUNNER_TEMP/fearlezz.md"
pictures_dir="$RUNNER_TEMP/fearlezz-pictures"

# The ffmpeg installed when the checks need it: John Van Sickle's static Linux builds of ffmpeg
# 7.0.2, as the ffmpeg-static project republishes them under its release tag b6.1.1.
ffmpeg_release="https://github.com/eugeneware/ffmpeg-static/releases/download/b6.1.1"
declare -A ffmpeg_sums=(
  [ffmpeg-x64]=bfe8a8fc511530457b528c48d77b5737527b504a3797a9bc4866aeca69c2dffa
  [ffprobe-x64]=25d9b6ccb05e3d9de9e04e31e2506d8dd7f9f0418981965ac6df12e8d3afd067
  [ffmpeg-arm64]=754a678672298bc68156adff58aa7385a592c2b30b1d0ae8750c45c915c4bac0
  [ffprobe-arm64]=2ab6aba60ee84412dff9188720703376cb4e7aaf7e0b5e43aa8249f2acae5bf8
)

# The marker lines Fearlezz writes (crates/fearlezz/src/github.rs).
status_marker="<!-- fearlezz -->"
waiting_marker="<!-- fearlezz-pictures "
approved_marker="<!-- fearlezz-approved "

output() { echo "$1=$2" >> "$GITHUB_OUTPUT"; }
field() { jq -r "$1 // empty" "$event"; }

# Whether the comment of the event starts with the command `/fearlezz approve`.
is_approval() {
  local first
  first=$(field .comment.body | head -n 1 | tr -d '\r')
  [ "$first" = "/fearlezz approve" ] || [[ "$first" == "/fearlezz approve "* ]]
}

task() {
  local name action
  name="${GITHUB_EVENT_NAME:-}"
  action=$(field .action)
  case "$name" in
    issue_comment)
      if [ "$(field .issue.pull_request.url)" != "" ] && is_approval; then
        output task approve
      else
        output task none
      fi
      ;;
    pull_request | pull_request_target)
      if [ "$action" = closed ]; then output task clean; else output task prove; fi
      ;;
    schedule) output task squash ;;
    *) output task prove ;;
  esac
}

install_fearlezz() {
  local dir="${FEARLEZZ_INSTALL_DIR:?}"
  local version="${VERSION:-latest}"
  # The action taken at a release tag installs that release, so the two always fit.
  if [ "$version" = latest ] && [[ "${ACTION_REF:-}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    version="$ACTION_REF"
  fi
  case "$version" in
    installed) dir=$(dirname "$(command -v fearlezz)") ;;
    source)
      cargo build --release --locked --manifest-path "$GITHUB_ACTION_PATH/Cargo.toml" \
        --target-dir "$RUNNER_TEMP/fearlezz-target"
      mkdir -p "$dir"
      cp "$RUNNER_TEMP/fearlezz-target/release/fearlezz" "$dir/"
      ;;
    latest) FEARLEZZ_INSTALL_DIR="$dir" sh "$GITHUB_ACTION_PATH/install.sh" ;;
    *) FEARLEZZ_VERSION="$version" FEARLEZZ_INSTALL_DIR="$dir" sh "$GITHUB_ACTION_PATH/install.sh" ;;
  esac
  echo "$dir" >> "$GITHUB_PATH"
  if ! "$dir/fearlezz" run --help | grep -q -- --pictures-url; then
    echo "::error::$("$dir/fearlezz" --version) is too old for this action, which needs fearlezz run --ci --pictures-url. Set version: to a newer release, or take the action at the tag of the release you install (fearlezz-dev/fearlezz@vX.Y.Z)."
    exit 1
  fi
}

needs_ffmpeg() {
  local need=false
  case "${FFMPEG:-auto}" in
    true) need=true ;;
    false) ;;
    *)
      if ! command -v ffmpeg > /dev/null && fearlezz setup needs | grep -qx ffmpeg; then
        need=true
      fi
      ;;
  esac
  output need "$need"
}

install_ffmpeg() {
  local dir="${FFMPEG_DIR:?}" arch tool
  case "$(uname -m)" in
    x86_64 | amd64) arch=x64 ;;
    aarch64 | arm64) arch=arm64 ;;
    *) echo "::warning::no static ffmpeg for $(uname -m); install ffmpeg before this action"; return 0 ;;
  esac
  mkdir -p "$dir"
  for tool in ffmpeg ffprobe; do
    if [ ! -x "$dir/$tool" ]; then
      curl -fsSL "$ffmpeg_release/$tool-linux-$arch.gz" -o "$dir/$tool.gz"
      echo "${ffmpeg_sums[$tool-$arch]}  $dir/$tool.gz" | sha256sum -c --quiet -
      gunzip -f "$dir/$tool.gz"
      chmod +x "$dir/$tool"
    fi
  done
  echo "$dir" >> "$GITHUB_PATH"
  "$dir/ffmpeg" -version | head -n 1
}

prove() {
  local pr same args
  pr=$(field .pull_request.number)
  same=$(jq -r '.pull_request.head.repo.full_name == .repository.full_name' "$event")
  args=(run --ci --comment "$comment_file" --pictures "$pictures_dir")
  # A fork's token cannot push, so its pictures stay in the artifact.
  if [ -n "$pr" ] && [ "$same" = true ]; then
    local url="$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/raw/$branch/pr-$pr/$GITHUB_RUN_ID-${GITHUB_RUN_ATTEMPT:-1}"
    output url "$url"
    args+=(--pictures-url "$url")
  fi
  fearlezz "${args[@]}"
}

# A blob-less clone of the pictures branch in $work, its tree in the index, its tip in $old
# (empty when the branch does not exist yet). Nothing is checked out, so it stays small however
# many pictures the branch holds.
open_branch() {
  work="$RUNNER_TEMP/fearlezz-pictures-git"
  rm -rf "$work"
  git init -q "$work"
  git -C "$work" config user.name "github-actions[bot]"
  git -C "$work" config user.email "41898282+github-actions[bot]@users.noreply.github.com"
  git -C "$work" remote add origin "$GITHUB_SERVER_URL/$GITHUB_REPOSITORY.git"
  # gh hands git the token, so it never lands in a URL or a file.
  git -C "$work" config credential.helper '!gh auth git-credential'
  old=""
  if git -C "$work" fetch -q --depth 1 --filter=blob:none origin "$branch" 2> /dev/null; then
    old=$(git -C "$work" rev-parse FETCH_HEAD)
    git -C "$work" read-tree FETCH_HEAD
  fi
}

# Commits the index of $work on top of $old (or as the only commit with "orphan") and pushes
# it; with "orphan" only while the branch is still at $old.
push_branch() {
  local message="$1" mode="${2:-}" tree commit
  tree=$(git -C "$work" write-tree)
  if [ "$mode" = orphan ] || [ -z "$old" ]; then
    commit=$(git -C "$work" commit-tree "$tree" -m "$message")
  else
    commit=$(git -C "$work" commit-tree "$tree" -p "$old" -m "$message")
  fi
  git -C "$work" push -q "--force-with-lease=refs/heads/$branch:$old" origin \
    "$commit:refs/heads/$branch"
}

publish() {
  local pr run attempt
  pr=$(field .pull_request.number)
  run="$GITHUB_RUN_ID-${GITHUB_RUN_ATTEMPT:-1}"
  # Other pull requests push to the same branch; a push that lost the race starts over.
  for attempt in 1 2 3; do
    open_branch
    # Only this run's pictures of this pull request stay.
    git -C "$work" rm -r -q --cached --ignore-unmatch "pr-$pr"
    mkdir -p "$work/pr-$pr"
    cp -R "$pictures_dir" "$work/pr-$pr/$run"
    git -C "$work" add "pr-$pr/$run"
    if push_branch "Pictures of pull request $pr, run $run"; then
      output pushed true
      echo "pictures at $GITHUB_SERVER_URL/$GITHUB_REPOSITORY/tree/$branch/pr-$pr/$run"
      return 0
    fi
    echo "push $attempt of the pictures failed"
  done
  output pushed false
  echo "::warning::could not push the pictures to $branch; the workflow needs contents: write"
}

# Who the token comments as: the workflow token cannot ask, and comments as github-actions[bot].
ci_login() {
  gh api user --jq .login 2> /dev/null || echo "github-actions[bot]"
}

# The id and body of the status comment this token wrote, as one JSON object, or nothing.
status_comment() {
  local pr="$1" login
  login=$(ci_login)
  gh api "$repo/issues/$pr/comments" --paginate \
    --jq ".[] | select(.user.login == \"$login\" and (.body | startswith(\"$status_marker\"))) | {id, body}" \
    | head -n 1
}

comment() {
  local pr found id url
  pr=$(field .pull_request.number)
  [ -s "$comment_file" ] || return 0
  if [ -n "${PICTURES:-}" ] && [ -n "${URL:-}" ] && [ "${PUSHED:-}" != true ]; then
    printf "\nThe pictures could not be put on the \`%s\` branch, so they do not show above; they are in the \`fearlezz-pictures\` artifact of the run.\n" \
      "$branch" >> "$comment_file"
  fi
  found=$(status_comment "$pr")
  if [ -n "$found" ]; then
    id=$(jq -r .id <<< "$found")
    url=$(gh api -X PATCH "$repo/issues/comments/$id" -F body=@"$comment_file" --jq .html_url)
  else
    url=$(gh api "$repo/issues/$pr/comments" -F body=@"$comment_file" --jq .html_url)
  fi
  output url "$url"
}

# Sets the fearlezz/pictures status of commit $1.
set_status() {
  local head="$1" state="$2" description="$3" target="${4:-}"
  local args=(-f state="$state" -f context=fearlezz/pictures -f description="${description:0:140}")
  if [ -n "$target" ]; then args+=(-f target_url="$target"); fi
  gh api "$repo/statuses/$head" "${args[@]}" > /dev/null
}

status() {
  local head checks
  head=$(field .pull_request.head.sha)
  # A run that ended before its report says nothing about the pictures.
  [ -s "$comment_file" ] || return 0
  if [ -n "${PICTURES:-}" ]; then
    checks="${PICTURES// /, }"
    set_status "$head" pending "changed pictures of $checks wait for /fearlezz approve" \
      "${COMMENT_URL:-}"
  else
    set_status "$head" success "no changed pictures"
  fi
}

approve() {
  local pr who kind url body said perm head found waiting checks headline approval id
  pr=$(field .issue.number)
  who=$(field .comment.user.login)
  kind=$(field .comment.user.type)
  url=$(field .comment.html_url)
  body=$(field .comment.body | tr -d '\r')
  reply() { gh api "$repo/issues/$pr/comments" -f body="$1" > /dev/null; }

  is_approval || return 0
  if [ "$kind" != User ]; then
    reply "Only a person can approve pictures, not @$who."
    return 0
  fi
  perm=$(gh api "$repo/collaborators/$who/permission" --jq .permission 2> /dev/null || echo none)
  case "$perm" in
    admin | write) ;;
    *)
      reply "@$who cannot approve pictures: that needs write access to this repository."
      return 0
      ;;
  esac
  said=$(printf '%s' "${body#/fearlezz approve}" | tr '\n' ' ' | sed 's/^ *//; s/ *$//')
  if [ -z "$said" ]; then
    reply "@$who, approving needs your own words: \`/fearlezz approve <what you think of the pictures>\`."
    return 0
  fi
  head=$(gh api "$repo/pulls/$pr" --jq .head.sha)
  found=$(status_comment "$pr")
  waiting=$(jq -r '.body // empty' <<< "${found:-null}" \
    | sed -n "s/^$waiting_marker\(.*\) -->\$/\1/p" | head -n 1)
  if [ -z "$waiting" ]; then
    reply "@$who, no changed pictures wait for approval on this pull request."
    return 0
  fi
  if [ "$(jq -r .head <<< "$waiting")" != "$head" ]; then
    reply "@$who, nothing to approve for ${head:0:7} yet: wait for its proof, then approve again."
    return 0
  fi
  checks=$(jq -r '.checks | map("`" + . + "`") | join(", ")' <<< "$waiting")

  set_status "$head" success "approved by $who" "$url"
  # The status comment says what was approved, in place of the line that said it waits.
  id=$(jq -r .id <<< "$found")
  headline=$(jq -r --arg by "$who" '.approved | gsub("\\{by\\}"; $by)' <<< "$waiting")
  jq -r .body <<< "$found" \
    | awk -v new="$headline" '!done && /^\*\*/ { print new; done = 1; next } { print }' \
    > "$RUNNER_TEMP/fearlezz-approved.md"
  gh api -X PATCH "$repo/issues/comments/$id" -F body=@"$RUNNER_TEMP/fearlezz-approved.md" > /dev/null
  approval=$(jq -c --arg by "$who" --arg said "$said" \
    '{head, fingerprint, checks, by: $by, said: $said}' <<< "$waiting" | sed 's/>/\\u003e/g')
  reply "@$who approved the changed pictures of $checks at ${head:0:7}:

> $said

A new push needs a new approval.
$approved_marker$approval -->"
}

# Removes the pictures folders named in "$@" and makes the branch a single commit again, so it
# never grows with the history of every run.
squash_without() {
  local attempt dir
  for attempt in 1 2 3; do
    open_branch
    if [ -z "$old" ]; then return 0; fi
    for dir in "$@"; do
      git -C "$work" rm -r -q --cached --ignore-unmatch "$dir"
    done
    if push_branch "Pictures of open pull requests" orphan; then
      echo "squashed $branch${*:+ without $*}"
      return 0
    fi
    echo "squash $attempt lost a race with another push"
  done
  echo "::warning::could not squash $branch"
}

clean() {
  squash_without "pr-$(field .pull_request.number)"
}

squash() {
  local closed=() dir state
  open_branch
  if [ -z "$old" ]; then return 0; fi
  for dir in $(git -C "$work" ls-tree --name-only FETCH_HEAD | grep '^pr-[0-9]*$' || true); do
    state=$(gh api "$repo/pulls/${dir#pr-}" --jq .state 2> /dev/null || echo closed)
    if [ "$state" != open ]; then closed+=("$dir"); fi
  done
  squash_without "${closed[@]}"
}

case "${1:-}" in
  task) task ;;
  install) install_fearlezz ;;
  needs-ffmpeg) needs_ffmpeg ;;
  ffmpeg) install_ffmpeg ;;
  prove) prove ;;
  publish) publish ;;
  comment) comment ;;
  status) status ;;
  approve) approve ;;
  clean) clean ;;
  squash) squash ;;
  none) ;;
  *) echo "usage: action.sh <step>; the steps are listed at the top of this file" >&2; exit 2 ;;
esac
