# claude-0019: pre-create writable state dirs in the core-runtime image

Infra review (journal 0015 on the infra branch): a Fargate ephemeral volume inherits ownership from the image path, so
the image must pre-create every writable path as the app uid. `/run/pulso-keys` already was; the exporter cursor dir
was not.

- Exporter state path: `PULSO_EXPORTER_STATE_DIR` (exporter `__main__`, `state.sqlite` inside it). The infra task
  definition sets it to `/var/lib/pulso-exporter`, so that is the path created (0700, uid 10001). `/tmp` stays a plain
  tmpfs/volume (world-writable by default).
- Dockerfile: `install -d -m 0700 -o 10001 /run/pulso-keys /var/lib/pulso-exporter`. No entrypoint change needed.
- RED first (against 894fa65-a7712b0): static Dockerfile check and the in-image `stat` check (700:10001) failed.
  A fresh rootless-Podman named volume happens to be writable even without the dir, so the write test alone does not
  discriminate locally; the stat check is the real guard (Fargate does inherit the image ownership).
- GREEN: rebuilt `localhost/pulso-core-runtime:894fa65-c096b2f`; `PULSO_TEST_IMAGE=... pytest tests/runtime/test_image.py`
  24 passed.
