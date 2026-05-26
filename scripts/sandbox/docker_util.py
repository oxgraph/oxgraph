"""Docker helpers for sandbox SQL benchmarks."""

from __future__ import annotations

import subprocess
import time
from pathlib import Path


def require_docker() -> None:
    proc = subprocess.run(["docker", "info"], capture_output=True)
    if proc.returncode != 0:
        raise RuntimeError("Docker is not available. Start Docker Desktop and retry.")


def ensure_graph_extension(container: str) -> None:
    docker_psql(container, "CREATE EXTENSION IF NOT EXISTS graph;")


def wait_for_postgres(container: str, *, timeout_s: float = 120.0) -> None:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        proc = subprocess.run(
            [
                "docker",
                "exec",
                container,
                "pg_isready",
                "-U",
                "postgres",
                "-d",
                "postgres",
            ],
            capture_output=True,
            text=True,
        )
        if proc.returncode == 0:
            return
        time.sleep(1)
    raise RuntimeError(f"Postgres in {container} did not become ready")


def docker_psql(container: str, sql: str, *, timeout: int | None = 600) -> str:
    cmd = [
        "docker",
        "exec",
        container,
        "psql",
        "-U",
        "postgres",
        "-d",
        "postgres",
        "-v",
        "ON_ERROR_STOP=1",
        "-tAc",
        sql,
    ]
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip() or proc.stdout.strip() or "psql failed")
    return proc.stdout


def ensure_container(
    *,
    name: str,
    image: str,
    host_port: int,
    build_context: Path | None = None,
    dockerfile: Path | None = None,
) -> None:
    inspect = subprocess.run(
        ["docker", "image", "inspect", image],
        capture_output=True,
    )
    if inspect.returncode != 0:
        if build_context is None or dockerfile is None:
            raise RuntimeError(f"Image {image} missing and no build context provided")
        subprocess.run(
            [
                "docker",
                "build",
                "-f",
                str(dockerfile),
                "-t",
                image,
                str(build_context),
            ],
            check=True,
        )

    running = subprocess.run(
        ["docker", "ps", "--format", "{{.Names}}"],
        capture_output=True,
        text=True,
        check=True,
    )
    if name in running.stdout.splitlines():
        wait_for_postgres(name)
        return

    exists = subprocess.run(
        ["docker", "ps", "-a", "--format", "{{.Names}}"],
        capture_output=True,
        text=True,
        check=True,
    )
    if name in exists.stdout.splitlines():
        subprocess.run(["docker", "start", name], check=True)
    else:
        subprocess.run(
            [
                "docker",
                "run",
                "--name",
                name,
                "-e",
                "POSTGRES_PASSWORD=postgres",
                "-p",
                f"{host_port}:5432",
                "-d",
                image,
            ],
            check=True,
        )
    wait_for_postgres(name)


def container_host_port(container: str) -> int:
    proc = subprocess.run(
        ["docker", "port", container, "5432/tcp"],
        capture_output=True,
        text=True,
        check=True,
    )
    line = proc.stdout.strip().splitlines()[0]
    return int(line.rsplit(":", 1)[-1])
