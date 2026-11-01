"""잘못된 패키지 변경을 차단하는 아키텍처 회귀 사례."""

from pathlib import Path
import tempfile
import unittest

from architecture import check


class ArchitectureTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.write("Cargo.toml", '''
[workspace]
members = ["crates/schema", "crates/compiler"]
[workspace.dependencies]
bci-schema = { path = "crates/schema" }
bci-compiler = { path = "crates/compiler" }
''')
        self.write("architecture.toml", '''
version = 1
package_root = "crates"
max_rust_path_parts = 1
[packages.bci-schema]
path = "crates/schema"
allows = []
[packages.bci-compiler]
path = "crates/compiler"
allows = ["bci-schema"]
''')
        for name in ("schema", "compiler"):
            self.write(f"crates/{name}/Cargo.toml", f'''
[package]
name = "bci-{name}"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
publish.workspace = true
[lib]
path = "lib.rs"
[lints]
workspace = true
''')
            self.write(f"crates/{name}/lib.rs", "//! 구조 검사 fixture.\n")
        self.append("crates/compiler/Cargo.toml", "\n[dependencies]\nbci-schema.workspace = true\n")

    def write(self, path, text):
        destination = self.root / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(text, encoding="utf-8")

    def append(self, path, text):
        destination = self.root / path
        destination.write_text(destination.read_text(encoding="utf-8") + text, encoding="utf-8")

    def test_allowed_workspace(self):
        self.assertEqual(check(self.root), [])

    def test_reverse_dependency(self):
        self.append("crates/schema/Cargo.toml", "\n[dependencies]\nbci-compiler.workspace = true\n")
        errors = check(self.root)
        self.assertTrue(any("허용 경계 위반" in error for error in errors))
        self.assertTrue(any("의존 순환" in error for error in errors))

    def test_target_dev_alias(self):
        self.append("Cargo.toml", 'compiler_alias = { package = "bci-compiler", path = "crates/compiler" }\n')
        self.append("crates/schema/Cargo.toml", '\n[target.\'cfg(unix)\'.dev-dependencies]\ncompiler_alias.workspace = true\n')
        self.assertTrue(any("허용 경계 위반" in error for error in check(self.root)))

    def test_build_dependency(self):
        self.append("crates/schema/Cargo.toml", '\n[build-dependencies]\nbci-compiler.workspace = true\n')
        self.assertTrue(any("허용 경계 위반" in error for error in check(self.root)))

    def test_unregistered_package(self):
        self.write("crates/extra/Cargo.toml", '[package]\nname = "extra"\n')
        self.assertTrue(any("실제 Cargo 패키지" in error for error in check(self.root)))

    def test_nested_source(self):
        self.write("crates/compiler/parse/nested.rs", "//! 중첩 소스 fixture.\n")
        self.assertTrue(any("깊이 초과" in error for error in check(self.root)))

    def test_missing_workspace_member(self):
        cargo = self.root / "Cargo.toml"
        cargo.write_text(cargo.read_text().replace(', "crates/compiler"', ''), encoding="utf-8")
        self.assertTrue(any("workspace members" in error for error in check(self.root)))

    def test_policy_cycle(self):
        policy = self.root / "architecture.toml"
        policy.write_text(policy.read_text().replace("allows = []", 'allows = ["bci-compiler"]'), encoding="utf-8")
        self.assertTrue(any("의존 순환" in error for error in check(self.root)))

    def test_direct_dependency_requires_central_definition(self):
        cargo = self.root / "crates/compiler/Cargo.toml"
        cargo.write_text(cargo.read_text().replace('bci-schema.workspace = true', 'bci-schema = { path = "../schema" }'), encoding="utf-8")
        self.assertTrue(any("상속한다" in error for error in check(self.root)))


if __name__ == "__main__":
    unittest.main()
