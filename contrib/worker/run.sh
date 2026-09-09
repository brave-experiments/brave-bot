#!/bin/bash
# Run a headless bravebot task against an S3 layout.
#
# Required env:
#   BUCKET      — S3 bucket name
#   TASK_ID     — task prefix under bravebot-tasks/
#   TASK_PROMPT — instruction for each turn (not PROMPT: that is zsh's PS1)
#
# Run mode (RUN_MODE):
#   once       — one turn, sync out, exit (default). Job Completes.
#   until-done — turn → sync → check outputs/DONE; stop when done, else continue
#                immediately (no long sleep). Requires a stop reason in DONE.
#   forever    — keep turning until the Job is deleted / hits activeDeadline,
#                or an operator drops /workspace/STOP.
#
# Optional env:
#   PAUSE_SECONDS — sleep between iterations in until-done / forever (default 0)
#   HOME          — container home (default /data); sessions under $HOME/.bravebot
#
# Completion (until-done):
#   After each turn the worker looks for outputs/DONE. The file must be non-empty;
#   its trimmed contents are the stop reason (synced to S3 with other outputs).
#   The prompt is appended with that contract automatically in until-done mode.
#
# Operator stop (any mode):
#   Create /workspace/STOP (optional body = reason), or delete the Job.

set -euo pipefail

cd /workspace
mkdir -p inputs outputs .bravebot/skills

: "${BUCKET:?BUCKET is required}"
: "${TASK_ID:?TASK_ID is required}"
: "${TASK_PROMPT:?TASK_PROMPT is required}"

export HOME="${HOME:-/data}"
mkdir -p "$HOME/.bravebot"
# Image ships default settings under /data; an emptyDir on HOME would wipe them — seed if missing.
if [[ ! -f "$HOME/.bravebot/settings.json" && -f /scripts/settings.json ]]; then
  cp /scripts/settings.json "$HOME/.bravebot/settings.json"
fi

RUN_MODE="${RUN_MODE:-once}"
PAUSE_SECONDS="${PAUSE_SECONDS:-0}"
TASK_PREFIX="s3://${BUCKET}/bravebot-tasks/${TASK_ID}"
SKILLS_PREFIX="s3://${BUCKET}/bravebot-skills"
DONE_FILE=outputs/DONE
STOP_FILE=/workspace/STOP

case "${RUN_MODE}" in
  once | until-done | forever) ;;
  *)
    echo "bravebot task ${TASK_ID}: invalid RUN_MODE=${RUN_MODE} (once|until-done|forever)" >&2
    exit 2
    ;;
esac

PROMPT="${TASK_PROMPT}"
if [[ "${RUN_MODE}" == "until-done" ]]; then
  PROMPT="${TASK_PROMPT}

When the task is fully complete, write a non-empty file at outputs/DONE whose
entire contents are a short stop reason (one line is enough). The worker stops
only when that file exists after a turn; do not leave it empty."
fi

echo "bravebot task ${TASK_ID}: starting (mode=${RUN_MODE} pause=${PAUSE_SECONDS}s)"

sync_in() {
  aws s3 sync "${TASK_PREFIX}/inputs/" inputs/ --only-show-errors
  aws s3 sync "${SKILLS_PREFIX}/" .bravebot/skills/ --only-show-errors
}

sync_out() {
  aws s3 sync outputs/ "${TASK_PREFIX}/outputs/" --only-show-errors
  aws s3 sync .bravebot/skills/ "${SKILLS_PREFIX}/" --only-show-errors
}

# Non-empty DONE body → reason. Empty or missing → not done.
done_reason() {
  if [[ ! -f "${DONE_FILE}" ]]; then
    return 1
  fi
  local reason
  reason="$(tr -d '\r' < "${DONE_FILE}" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' | tr '\n' ' ' | sed 's/[[:space:]]*$//')"
  if [[ -z "${reason}" ]]; then
    return 1
  fi
  printf '%s' "${reason}"
}

stop_requested() {
  [[ -f "${STOP_FILE}" ]]
}

stop_reason_from_file() {
  if [[ -f "${STOP_FILE}" ]]; then
    local reason
    reason="$(tr -d '\r' < "${STOP_FILE}" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' | tr '\n' ' ' | sed 's/[[:space:]]*$//')"
    if [[ -n "${reason}" ]]; then
      printf '%s' "${reason}"
      return 0
    fi
  fi
  printf '%s' "operator stop (${STOP_FILE})"
}

finish() {
  local reason="$1"
  local code="${2:-0}"
  printf '%s\n' "${reason}" > outputs/STOP_REASON
  sync_out || true
  echo "bravebot task ${TASK_ID}: stopped: ${reason}"
  exit "${code}"
}

iteration=0
while true; do
  if stop_requested; then
    finish "$(stop_reason_from_file)" 0
  fi

  iteration=$((iteration + 1))
  sync_in

  # Do not pass inputs via --file: that inlines every byte into the first request.
  # Large datasets belong on disk under inputs/ for the agent to read with tools.
  echo "bravebot task ${TASK_ID}: turn ${iteration} ($(date -u +%Y-%m-%dT%H:%M:%SZ))"
  turn_status=0
  bravebot --dangerously-skip-permissions -p "${PROMPT}" || turn_status=$?

  sync_out

  if stop_requested; then
    finish "$(stop_reason_from_file)" 0
  fi

  case "${RUN_MODE}" in
    once)
      if [[ "${turn_status}" -ne 0 ]]; then
        finish "turn failed (exit ${turn_status})" "${turn_status}"
      fi
      finish "once: turn completed" 0
      ;;
    until-done)
      if reason="$(done_reason)"; then
        finish "done: ${reason}" 0
      fi
      if [[ "${turn_status}" -ne 0 ]]; then
        echo "bravebot task ${TASK_ID}: turn ${iteration} failed (exit ${turn_status}); no DONE yet, continuing"
      else
        echo "bravebot task ${TASK_ID}: turn ${iteration} finished; outputs/DONE not set, continuing"
      fi
      ;;
    forever)
      if [[ "${turn_status}" -ne 0 ]]; then
        echo "bravebot task ${TASK_ID}: turn ${iteration} failed (exit ${turn_status}); continuing"
      fi
      ;;
  esac

  if [[ "${PAUSE_SECONDS}" -gt 0 ]]; then
    echo "bravebot task ${TASK_ID}: pause ${PAUSE_SECONDS}s"
    sleep "${PAUSE_SECONDS}"
  fi
done
