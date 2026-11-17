#!/usr/bin/env bash
# Runs before each job as ACTIONS_RUNNER_HOOK_JOB_STARTED. GitHub gives a queued
# job to any idle runner whose labels match, so this runner may have received a
# job from a workflow run that Runlet didn't accept for its trust class. Runlet
# records each accepted run as a file in the allowed-runs directory, possibly a
# few seconds after GitHub assigned the job, so wait for it before refusing.
#
# Arguments, for tests only: allowed-runs directory, seconds to wait.
set -u

allowed_runs_dir="${1:-/etc/runlet/allowed-runs}"
wait_seconds="${2:-60}"
run_id="${GITHUB_RUN_ID:-}"

if [[ "${run_id}" =~ ^[0-9]+$ ]]; then
  for ((waited = 0; ; waited++)); do
    if [[ -e "${allowed_runs_dir}/${run_id}" ]]; then
      exit 0
    fi
    if ((waited >= wait_seconds)); then
      break
    fi
    sleep 1
  done
fi

echo "::error::Runlet did not start this runner for workflow run ${run_id:-unknown}. Another job took it; re-run this job if Runlet should run it."

# A failing hook alone doesn't stop the job: steps with `if: always()` and
# action pre steps still run. Stop the worker process that runs this job.
pid=$$
while ((pid > 1)); do
  if [[ "$(cat "/proc/${pid}/comm" 2>/dev/null)" == "Runner.Worker" ]]; then
    kill -KILL "${pid}"
    exit 1
  fi
  stat="$(cat "/proc/${pid}/stat" 2>/dev/null)" || break
  read -r -a fields <<<"${stat##*) }"
  pid="${fields[1]}"
done

echo "::error::Runlet could not find the runner worker process to stop."
exit 1
