"""개발 환경과 CI가 공유하는 workspace 검사 진입점."""

from pathlib import Path
import os
import subprocess
import sys


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    steps = [
        ("아키텍처", [sys.executable, "tools/architecture.py"]),
        ("아키텍처 회귀", [sys.executable, "-m", "unittest", "discover", "-s", "tools", "-p", "test_*.py"]),
        ("Rust 서식", ["cargo", "fmt", "--all", "--", "--check"]),
        ("Clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--all-features", "--offline", "--locked", "--", "-D", "warnings"]),
        ("Rust 테스트", ["cargo", "test", "--workspace", "--all-features", "--offline", "--locked"]),
        ("Rust 문서", ["cargo", "doc", "--workspace", "--all-features", "--no-deps", "--offline", "--locked"]),
        ("CLI·확장 번들 빌드", ["cargo", "build", "-p", "bci-cli", "--bins", "--examples", "--offline", "--locked"]),
    ]
    environment = os.environ.copy()
    environment["RUSTDOCFLAGS"] = (environment.get("RUSTDOCFLAGS", "") + " -D warnings").strip()
    completed = 0
    total = len(steps) + 1
    try:
        for label, command in steps:
            print(f"[{completed + 1}/{total}] {label}", flush=True)
            subprocess.run(command, cwd=root, env=environment, check=True)
            completed += 1
        print(f"[{total}/{total}] CLI 동작", flush=True)
        metadata = subprocess.run(
            ["cargo", "metadata", "--no-deps", "--format-version", "1", "--offline", "--locked"],
            cwd=root, env=environment, check=True, capture_output=True, text=True,
        )
        import json
        target = Path(json.loads(metadata.stdout)["target_directory"])
        executable = target / "debug" / ("bci.exe" if os.name == "nt" else "bci")
        for arguments, expected in (
            (["--help"], 0), (["--version"], 0), (["unknown-command"], 2),
            (["check", "examples/typed.bci"], 0),
            (["check", "examples/multi.bci"], 0),
            (["check", "examples/channels.bci"], 0),
            (["check", "examples/spectral.bci"], 0),
            (["check", "examples/iir.bci"], 0),
            (["check", "examples/resample.bci"], 0),
            (["check", "examples/multirate.bci"], 0),
            (["check", "examples/learning.bci"], 0),
            (["check", "examples/selection.bci"], 0),
            (["explain", "examples/typed.bci", "--view", "types"], 0),
        ):
            result = subprocess.run([str(executable), *arguments], cwd=root, capture_output=True, text=True)
            if result.returncode != expected or not (result.stdout or result.stderr).strip():
                raise RuntimeError(f"CLI {arguments}: 종료 코드 {result.returncode}, 기대값 {expected}")
        bundle = target / "debug" / "examples" / ("bci-lab.exe" if os.name == "nt" else "bci-lab")
        for arguments in (["extensions"], ["extensions", "check"], ["check", "examples/extension.bci"], ["check", "examples/stateful.bci"]):
            result = subprocess.run([str(bundle), *arguments], cwd=root, capture_output=True, text=True)
            if result.returncode != 0 or not result.stdout.strip():
                raise RuntimeError(f"확장 번들 {arguments}: {result.stderr}")
        completed += 1
    except (OSError, subprocess.CalledProcessError, RuntimeError) as error:
        print(f"검사 실패: {error}\n통과 {completed}/{total}", file=sys.stderr)
        return 1
    print(f"전체 검사 통과: {completed}/{total}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
