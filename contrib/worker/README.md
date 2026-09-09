# contrib/worker

A Docker image that runs bravebot against an S3 layout. Useful for Kubernetes
Jobs or any scheduler that can set env vars and give the container AWS
credentials. Not built, shipped, or run by CI — same footing as the rest of
[`contrib/`](../README.md).

It is a **general-purpose** worker: common CLI tools, Node, and a practical
Python stack so ad-hoc tasks are not blocked on missing basics. It is not a
full ML/GPU image (no torch/CUDA); derive from this if a task needs that.

## Run modes (`RUN_MODE`)

| Mode | Behaviour |
| --- | --- |
| `once` (default) | One turn, sync outputs, exit. Job Completes. |
| `until-done` | Turn → sync → check `outputs/DONE`; stop when that file has a reason, else continue immediately. |
| `forever` | Keep turning until the Job is deleted / hits its deadline, or `/workspace/STOP` appears. |

Optional `PAUSE_SECONDS` (default `0`) sleeps between iterations in `until-done` / `forever` only. There is no hour-long default interval.

### Stopping

- **`until-done`:** the agent writes a **non-empty** `outputs/DONE` whose contents are the stop reason. The worker appends that contract to the prompt. Reason is also copied to `outputs/STOP_REASON` on exit.
- **Any mode:** create `/workspace/STOP` (optional body = reason), or delete the Job / hit `activeDeadlineSeconds`.
- Outputs (including partial work) are synced after every turn, including failed ones.

## What each turn does

1. Sync `s3://$BUCKET/bravebot-tasks/$TASK_ID/inputs/` → `/workspace/inputs/`
2. Sync `s3://$BUCKET/bravebot-skills/` → `/workspace/.bravebot/skills/`
3. Run `bravebot --dangerously-skip-permissions -p "$TASK_PROMPT"`
   (inputs stay on disk for tools — they are not inlined with `--file`)
4. Sync `/workspace/outputs/` and skills back to S3
5. Apply the run-mode stop rule above

## Tooling (baked in)

- CLI: `bash`, `rg`, `fd`, `jq`, `git`, `curl`/`wget`, `sqlite3`, archives, `build-essential`
- Node: `nodejs` / `npm` (bookworm)
- Python: `numpy`, `pandas`, `scipy`, `sklearn`, `requests`, `yaml`, `bs4`, `lxml`, `PIL`, `openpyxl`, plus `duckdb` / `jsonlines` / `httpx` / `rich` via pip
- AWS: `aws` CLI (for the sync loop and agent use)

## Build

From the repository root (needs a CLI image first):

```sh
make docker-image
docker build -f contrib/worker/Dockerfile \
  --build-arg BASE_IMAGE=bravebot:0.4.0 \
  -t bravebot-worker:0.4.0 \
  .
```

## Run

```sh
docker run --rm \
  -e BUCKET=my-bucket \
  -e TASK_ID=run-42 \
  -e TASK_PROMPT='Review inputs/. Write results under outputs/.' \
  -e RUN_MODE=until-done \
  -e AWS_ACCESS_KEY_ID -e AWS_SECRET_ACCESS_KEY -e AWS_REGION \
  bravebot-worker:0.4.0
```

Or schedule the same image as a Kubernetes Job with IRSA / a service account.

## Layout on S3

```
s3://$BUCKET/
  bravebot-skills/<name>/SKILL.md
  bravebot-tasks/$TASK_ID/inputs/
  bravebot-tasks/$TASK_ID/outputs/   # includes DONE / STOP_REASON when used
```
