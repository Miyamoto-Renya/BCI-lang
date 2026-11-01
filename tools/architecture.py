"""Workspace 패키지 경계와 Rust 파일 깊이를 검사한다."""

from pathlib import Path
import sys
import tomllib


def read_toml(path: Path) -> dict:
    """UTF-8 TOML 파일을 읽는다."""
    return tomllib.loads(path.read_text(encoding="utf-8"))


def dependency_tables(manifest: dict):
    """일반·개발·빌드 및 target별 의존성을 반환한다."""
    groups = [("package", manifest), *manifest.get("target", {}).items()]
    for context, group in groups:
        for kind in ("dependencies", "dev-dependencies", "build-dependencies"):
            for alias, spec in group.get(kind, {}).items():
                yield f"{context}.{kind}", alias, spec


def check_cycles(graph: dict[str, set[str]]) -> list[str]:
    """의존 경로를 순회해 순환의 경로를 반환한다."""
    errors = []
    visited = set()
    active = []

    def visit(node):
        if node in active:
            start = active.index(node)
            errors.append("의존 순환: " + " -> ".join(active[start:] + [node]))
            return
        if node in visited:
            return
        active.append(node)
        for dependency in sorted(graph.get(node, set())):
            visit(dependency)
        active.pop()
        visited.add(node)

    for node in sorted(graph):
        visit(node)
    return errors


def check(root: Path) -> list[str]:
    """정책과 실제 manifest·소스에서 위반 목록을 계산한다."""
    root = root.resolve()
    policy = read_toml(root / "architecture.toml")
    workspace = read_toml(root / "Cargo.toml")["workspace"]
    packages = policy["packages"]
    errors = []
    if policy.get("version") != 1:
        errors.append("architecture.toml은 정책 version=1을 요구한다.")
    package_root = policy["package_root"]
    declared_paths = [record["path"] for record in packages.values()]
    members = workspace["members"]
    if len(declared_paths) != len(set(declared_paths)):
        errors.append("여러 패키지가 같은 경로를 선언했다.")
    if len(members) != len(set(members)) or set(members) != set(declared_paths):
        errors.append("workspace members와 architecture packages의 경로를 일치시킨다.")

    discovered = {
        str(path.parent.relative_to(root))
        for path in (root / package_root).rglob("Cargo.toml")
        if "target" not in path.relative_to(root / package_root).parts
    }
    if discovered != set(declared_paths):
        errors.append("실제 Cargo 패키지와 정책 경로 불일치: " + str(sorted(discovered ^ set(declared_paths))))

    allowed = {name: set(record["allows"]) for name, record in packages.items()}
    actual = {name: set() for name in packages}
    for name, dependencies in allowed.items():
        for dependency in dependencies - packages.keys():
            errors.append(f"{name}: 정책에 등록할 의존 패키지 {dependency}")
    errors.extend(check_cycles(allowed))

    common_dependencies = workspace.get("dependencies", {})
    paths_to_names = {(root / record["path"]).resolve(): name for name, record in packages.items()}
    for name, record in packages.items():
        relative = Path(record["path"])
        if len(relative.parts) != 2 or relative.parts[0] != package_root or ".." in relative.parts:
            errors.append(f"{name}: 패키지는 {package_root}/이름 깊이에 배치한다.")
            continue
        directory = root / relative
        if not (directory / "Cargo.toml").is_file():
            errors.append(f"{name}: Cargo.toml 경로를 확인한다.")
            continue
        manifest = read_toml(directory / "Cargo.toml")
        package = manifest["package"]
        if package.get("name") != name:
            errors.append(f"{name}: manifest package.name 불일치")
        for field in ("version", "edition", "rust-version", "publish"):
            if package.get(field) != {"workspace": True}:
                errors.append(f"{name}: {field}를 workspace에서 상속한다.")
        if manifest.get("lints") != {"workspace": True}:
            errors.append(f"{name}: workspace lints 상속을 적용한다.")
        targets = [manifest["lib"]] if "lib" in manifest else []
        for kind in ("bin", "test", "bench", "example"):
            targets.extend(manifest.get(kind, []))
        if not targets:
            errors.append(f"{name}: lib 또는 bin target의 파일 경로를 선언한다.")
        for target in targets:
            path = target.get("path", "")
            if not path or len(Path(path).parts) != 1 or not (directory / path).is_file():
                errors.append(f"{name}: target은 패키지 루트의 실제 파일을 지정한다: {path}")
        for source in directory.rglob("*.rs"):
            parts = source.relative_to(directory).parts
            if "target" in parts:
                continue
            if len(parts) > policy["max_rust_path_parts"]:
                errors.append(f"{name}: Rust 파일 깊이 초과: {source.relative_to(root)}")

        for context, alias, spec in dependency_tables(manifest):
            inherited = isinstance(spec, dict) and spec.get("workspace") is True
            if inherited:
                if alias not in common_dependencies:
                    errors.append(f"{name}: workspace dependency 누락: {alias}")
                    continue
                resolved = common_dependencies[alias]
                base = root
            else:
                errors.append(f"{name}: {context}의 {alias}를 workspace.dependencies에서 상속한다.")
                resolved = spec
                base = directory
            dependency = resolved.get("package", alias) if isinstance(resolved, dict) else alias
            if isinstance(resolved, dict) and "path" in resolved:
                dependency_path = (base / resolved["path"]).resolve()
                canonical = paths_to_names.get(dependency_path)
                if canonical is None:
                    errors.append(f"{name}: 경로 의존성을 정책에 등록한다: {alias}")
                    continue
                if canonical != dependency:
                    errors.append(f"{name}: 의존성 이름·경로 불일치: {alias} -> {canonical}")
                dependency = canonical
            elif dependency in packages:
                errors.append(f"{name}: 내부 의존성 {dependency}는 workspace path로 연결한다.")
            if dependency in packages:
                actual[name].add(dependency)
                if dependency not in allowed[name]:
                    errors.append(f"{name}: 허용 경계 위반 ({context}): {dependency}")
    errors.extend(check_cycles(actual))
    return errors


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    try:
        errors = check(root)
    except (OSError, KeyError, TypeError, ValueError) as error:
        print(f"아키텍처 입력 오류: {error}", file=sys.stderr)
        return 1
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    count = len(read_toml(root / "architecture.toml")["packages"])
    print(f"아키텍처 검사 통과: {count}개 패키지, 의존 방향·순환·상속·파일 깊이")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
