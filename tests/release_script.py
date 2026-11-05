import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
VERSION = re.search(r'(?ms)\[package\]\n.*?^version = "([^"]+)"', (ROOT / "Cargo.toml").read_text())[1]
NEXT_VERSION = str(int(VERSION.split(".")[0]) + 1) + ".3.4"
FAKE = r'''
import hashlib, io, json, os, pathlib, sys, tarfile
tool, args = pathlib.Path(sys.argv[1]).name, sys.argv[2:]
root = pathlib.Path(os.environ["RELEASE_TEST_ROOT"])
with (root / "calls").open("a") as log:
    log.write(json.dumps([tool, *args]) + "\n")
failure = os.environ.get("RELEASE_TEST_FAILURE", "")
state_path = root / "state"
state = json.loads(state_path.read_text())
def save(): state_path.write_text(json.dumps(state))
def fail(): sys.exit(1)
if tool == "git":
    tap = args[:1] == ["-C"]
    if tap:
        assert args[1] == str(root.parent / "homebrew-tap"), args
        args = args[2:]
    if args == ["rev-parse", "--show-toplevel"]:
        print(root.parent if tap and failure == "tap-wrong-root" else root.parent / "homebrew-tap" if tap else root)
    elif args == ["branch", "--show-current"]: print("main")
    elif args == ["status", "--porcelain"]:
        if (failure == "dirty" and not tap) or (failure == "tap-dirty" and tap): print(" M unrelated")
    elif args[:2] == ["remote", "get-url"]: print(os.environ["RELEASE_TEST_TAP_ORIGIN"] if tap else "git@github.com:noih/reswho.git")
    elif args == ["rev-parse", "HEAD"]: print("taphead" if tap else "releasehead")
    elif args[:2] == ["merge-base", "--is-ancestor"]: pass
    elif args[:3] == ["rev-parse", "-q", "--verify"]: sys.exit(0 if failure == "local-tag" else 1)
    elif args[:2] == ["diff", "--quiet"]:
        sys.exit(0 if all((root / p).read_text() == v for p,v in state["original"].items()) else 1)
    elif args[:1] == ["commit"]:
        if not tap: state["original"] = {p:(root / p).read_text() for p in state["original"]}; save()
    elif args[:1] == ["push"]:
        if failure == "push": fail()
    elif args[:1] not in (["add"], ["tag"]): raise AssertionError(args)
elif tool == "gh":
    if args[:2] == ["auth", "status"]: pass
    elif args[:1] == ["api"]:
        path = args[1]
        if "/actions/runs?" in path:
            if failure == "actions-403":
                print("gh: Forbidden (HTTP 403)", file=sys.stderr); fail()
            print('{}')
        elif "/actions/runs/" in path:
            print(json.dumps({"head_sha":"releasehead","status":"completed","conclusion":"failure" if failure == "actions" else "success"}))
        elif "homebrew-tap" in path: print("drift" if failure == "tap-drift" else "taphead")
        elif path.endswith("heads/main"): print("basehead")
        else:
            if failure == "existing-release": sys.exit(0)
            print("gh: Forbidden (HTTP 403)" if failure == "403" else "gh: Not Found (HTTP 404)", file=sys.stderr)
            sys.exit(1)
    elif args[:2] == ["run", "list"]: print("4242")
    elif args[:2] == ["run", "watch"]:
        if failure == "actions": fail()
    else: raise AssertionError(args)
elif tool == "brew":
    assert args[:1] == ["style"], args
    assert pathlib.Path(args[1]).is_absolute() and pathlib.Path(args[1]).is_file(), args
    if failure == "style": fail()
elif tool == "cargo":
    if (failure == "gate" and args[:1] == ["test"]) or (failure == "publish" and args == ["publish", "--locked"]): fail()
elif tool == "python3":
    if args[:1] == ["-"] or args[:1] == ["scripts/release-notes"]: os.execv(sys.executable, [sys.executable, *args])
    if args[0] not in ("scripts/package-release", "tests/release_package.py", "tests/release_script.py"): raise AssertionError(args)
    if args[0] == "scripts/package-release" and len(args) == 4:
        import string
        output = pathlib.Path(args[3]); output.mkdir()
        (output / "reswho.rb").write_text(string.Template((root / "packaging/reswho.rb.template").read_text()).substitute(TAG=args[1],SHA256="0"*64))
elif tool == "curl":
    destination = pathlib.Path(args[args.index("-o") + 1])
    name = destination.name
    if name.endswith(".tar.gz"):
        with tarfile.open(destination, "w:gz") as archive:
            for member in ("reswho", "LICENSE", "README.md"):
                info = tarfile.TarInfo(member); info.size=1
                archive.addfile(info, io.BytesIO(b"x"))
    else:
        archive = next(destination.parent.glob("*.tar.gz"))
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        if name.endswith(".sha256"):
            destination.write_text(("0"*64 if failure == "checksum" else digest) + "  " + archive.name + "\n")
        else:
            import string
            tag = archive.name[len("reswho-"):-len("-aarch64-apple-darwin.tar.gz")]
            destination.write_text("wrong formula" if failure == "formula" else string.Template((root / "packaging/reswho.rb.template").read_text()).substitute(TAG=tag,SHA256=digest))
elif tool not in ("ruby", "shasum", "sleep"): raise AssertionError(tool)
'''


class ReleaseScriptTests(unittest.TestCase):
    def run_release(self, version=VERSION, failure="", tap_origin="git@github.com:noih/homebrew-tap.git", changelog=None):
        temporary = tempfile.TemporaryDirectory(prefix="reswho-release-script-")
        self.addCleanup(temporary.cleanup)
        owned = Path(temporary.name).resolve()
        root = owned / "reswho"
        root.mkdir()
        for name in ("Cargo.toml", "Cargo.lock", "packaging/reswho.rb.template", "scripts/release", "scripts/release-notes"):
            target = root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)
        (root / "CHANGELOG.md").write_text(changelog if changelog is not None else "## " + version + "\n\nRelease notes.\n")
        if failure != "missing-tap":
            (owned / "homebrew-tap/Formula").mkdir(parents=True)
        managed_formula = owned / "managed-tap/Formula/reswho.rb"
        managed_formula.parent.mkdir(parents=True)
        managed_formula.write_text("managed tap sentinel\n")
        (root / "tmp").mkdir()
        (root / "bin").mkdir()
        (root / "fake.py").write_text(FAKE)
        for tool in ("git", "gh", "cargo", "python3", "brew", "curl", "ruby", "shasum", "sleep"):
            wrapper = root / "bin" / tool
            wrapper.write_text('#!/bin/sh\nexec ' + sys.executable + ' "' + str(root / "fake.py") + '" "$0" "$@"\n')
            wrapper.chmod(0o755)
        original = {p:(root / p).read_text() for p in ("Cargo.toml", "Cargo.lock")}
        (root / "state").write_text(json.dumps({"original": original}))
        env = dict(os.environ, PATH=str(root / "bin") + ":/usr/bin:/bin", HOME=str(root), TMPDIR=str(root / "tmp"), RELEASE_TEST_ROOT=str(root), RELEASE_TEST_FAILURE=failure, RELEASE_TEST_TAP_ORIGIN=tap_origin, CARGO_REGISTRY_TOKEN="NEVER_LOG_THIS_TOKEN")
        result = subprocess.run(["/bin/bash", str(root / "scripts/release"), version], cwd=root, env=env, text=True, capture_output=True, timeout=30)
        calls = [json.loads(line) for line in (root / "calls").read_text().splitlines()] if (root / "calls").exists() else []
        self.assertEqual(managed_formula.read_text(), "managed tap sentinel\n")
        self.assertFalse(any("managed-tap" in arg or "noih/tap" in arg for call in calls for arg in call))
        self.assertNotIn("NEVER_LOG_THIS_TOKEN", result.stdout + result.stderr + json.dumps(calls))
        self.assertEqual(list((root / "tmp").iterdir()), [], "owned script temporary directory leaked")
        return result, calls, root, original

    def test_tap_origin_variants_print_explicit_ssh_push(self):
        for origin in ("https://github.com/noih/homebrew-tap", "https://github.com/noih/homebrew-tap.git", "git@github.com:noih/homebrew-tap", "git@github.com:noih/homebrew-tap.git"):
            with self.subTest(origin=origin):
                result, calls, root, _ = self.run_release(tap_origin=origin)
                self.assertEqual(result.returncode, 0, result.stderr)
                tap = root.parent / "homebrew-tap"
                push_line = next(line for line in result.stdout.splitlines() if line.startswith("  git -C "))
                self.assertEqual(shlex.split(push_line), ["git", "-C", str(tap), "push", "git@github.com:noih/homebrew-tap.git", "main"])
                self.assertIn(["git", "-C", str(tap), "add", "Formula/reswho.rb"], calls)
                self.assertIn(["git", "-C", str(tap), "commit", "-m", "reswho " + VERSION], calls)
                self.assertIn(["brew", "style", str(tap / "Formula/reswho.rb")], calls)
                self.assertNotIn("push origin", result.stdout)
                self.assertTrue((root.parent / "homebrew-tap/Formula/reswho.rb").exists())
                self.assertFalse(any(c[:3] == ["git", "-C", str(root.parent / "homebrew-tap")] and c[3:4] == ["push"] for c in calls))

    def test_wrong_tap_origins_stop_before_version_or_publication(self):
        for origin in ("https://github.com/noih/homebrew-taps", "https://github.com/other/homebrew-tap", "https://example.com/noih/homebrew-tap", "https://github.com.evil/noih/homebrew-tap", "https://github.com/noih/homebrew-tap.git/extra", "https://github.com/noih/homebrew-tap-extra", "git@github.com:other/homebrew-tap.git", "git@example.com:noih/homebrew-tap.git", "git@github.com:noih/homebrew-tap.git/extra", "https://NEVER_LOG_THIS_TOKEN@github.com/noih/homebrew-tap"):
            with self.subTest(origin=origin):
                result, calls, root, original = self.run_release(NEXT_VERSION, tap_origin=origin)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("release: tap origin is not noih/homebrew-tap", result.stderr)
                for path, content in original.items():
                    self.assertEqual((root / path).read_text(), content)
                self.assertFalse(any(c[0] == "cargo" or c[:2] in (["git", "tag"], ["git", "push"]) for c in calls))
                self.assertEqual(list((root.parent / "homebrew-tap/Formula").iterdir()), [])

    def test_same_version_runs_without_empty_bump_commit(self):
        result, calls, root, original = self.run_release()
        self.assertEqual(result.returncode, 0, result.stderr)
        for name, content in original.items():
            self.assertEqual((root / name).read_text(), content)
        self.assertNotIn(["git", "commit", "-m", "chore(release): prepare " + VERSION], calls)
        gate = calls.index(["cargo", "test", "--locked", "--", "--test-threads=1"])
        tag = calls.index(["git", "tag", "-a", "v" + VERSION, "-m", "reswho " + VERSION])
        push = calls.index(["git", "push", "origin", "main", "v" + VERSION])
        publish = calls.index(["cargo", "publish", "--locked"])
        download = next(i for i,c in enumerate(calls) if c[0] == "curl")
        self.assertTrue(gate < tag < push < publish < download)
        style = next(i for i,c in enumerate(calls) if c[:2] == ["brew", "style"])
        self.assertTrue(gate < style < tag < push)
        self.assertEqual(Path(calls[style][2]).parts[-2:], ("Formula", "reswho.rb"))
        listing = next(c for c in calls if c[:3] == ["gh", "run", "list"])
        self.assertIn("releasehead", listing)
        self.assertIn("v" + VERSION, listing)
        self.assertTrue((root.parent / "homebrew-tap/Formula/reswho.rb").exists())
        self.assertEqual(sum(c[:2] == ["git", "push"] for c in calls), 1)

    def test_argument_changes_package_lock_and_same_tree_example(self):
        result, calls, root, _ = self.run_release(NEXT_VERSION)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('version = "' + NEXT_VERSION + '"', (root / "Cargo.toml").read_text())
        self.assertIn('name = "reswho"\nversion = "' + NEXT_VERSION + '"', (root / "Cargo.lock").read_text())
        self.assertIn(["git", "commit", "-m", "chore(release): prepare " + NEXT_VERSION], calls)

    def test_invalid_notes_stop_before_version_changes_or_public_actions(self):
        for changelog in ("## 1.0.0\nOld.\n", "## 1.0.1\n \n", "## 1.0.1\nFix.\n## 1.0.1\nOther.\n"):
            with self.subTest(changelog=changelog):
                result, calls, root, original = self.run_release("1.0.1", changelog=changelog)
                self.assertNotEqual(result.returncode, 0)
                for path, content in original.items():
                    self.assertEqual((root / path).read_text(), content)
                self.assertFalse(any(c[0] in ("gh", "cargo", "curl") or c[:2] in (["git", "commit"], ["git", "tag"], ["git", "push"]) for c in calls))

    def test_workflow_draft_uses_exact_extracted_body_and_rejects_invalid_notes(self):
        source = (ROOT / ".github/workflows/release.yml").read_text()
        draft = textwrap.dedent(source.split("      - name: Create a new draft only\n", 1)[1].split("        run: |\n", 1)[1].split("      - run:", 1)[0])
        with tempfile.TemporaryDirectory(prefix="reswho-release-draft-") as temporary:
            root = Path(temporary)
            (root / "scripts").mkdir()
            shutil.copyfile(ROOT / "scripts/release-notes", root / "scripts/release-notes")
            (root / "gh").write_text('#!' + sys.executable + '\nimport pathlib, sys\nroot = pathlib.Path(__file__).parent\nwith (root / "calls").open("a") as log: log.write(repr(sys.argv[1:]) + "\\n")\nif sys.argv[1] == "api":\n print("HTTP 404", file=sys.stderr)\n sys.exit(1)\nassert sys.argv[1:8] == ["release", "create", "v1.0.1", "--verify-tag", "--draft", "--title", "v1.0.1"]\n(root / "body").write_bytes(pathlib.Path(sys.argv[sys.argv.index("--notes-file") + 1]).read_bytes())\n')
            (root / "gh").chmod(0o755)
            env = dict(os.environ, PATH=temporary + ":" + os.environ["PATH"], TAG="v1.0.1", GH_REPO="noih/reswho", RUNNER_TEMP=temporary)
            body = b"### Upgrade effort\n\nMicro: no user changes needed.\n\nUpdated the README with a clearer introduction, quick-start examples, and links to detailed guides.\n\nNo functional changes.\n"
            (root / "CHANGELOG.md").write_bytes((ROOT / "CHANGELOG.md").read_bytes())
            result = subprocess.run(["/bin/bash", "-euo", "pipefail", "-c", draft], cwd=root, env=env, capture_output=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual((root / "body").read_bytes(), body)
            for changelog in ("## 1.0.0\nOld.\n", "## 1.0.1\n", "## 1.0.1\nFix.\n## 1.0.1\nOther.\n"):
                (root / "calls").unlink(missing_ok=True)
                (root / "CHANGELOG.md").write_text(changelog)
                result = subprocess.run(["/bin/bash", "-euo", "pipefail", "-c", draft], cwd=root, env=env, capture_output=True, timeout=5)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((root / "calls").exists())

    def test_preflight_and_gate_failures_cannot_push(self):
        for failure in ("dirty", "403", "actions-403", "local-tag", "existing-release", "missing-tap", "tap-wrong-root", "tap-dirty", "tap-drift", "gate", "style"):
            with self.subTest(failure=failure):
                result, calls, root, original = self.run_release(NEXT_VERSION, failure)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(any(c[:2] == ["git", "push"] or c == ["cargo", "publish", "--locked"] for c in calls))
                if failure not in ("gate", "style"):
                    for path, content in original.items():
                        self.assertEqual((root / path).read_text(), content)
                    self.assertFalse(any(c[0] in ("cargo", "curl") or c[:2] == ["git", "tag"] for c in calls))
                    self.assertFalse((root.parent / "homebrew-tap/Formula/reswho.rb").exists())
                if failure == "missing-tap":
                    self.assertIn("create the sibling homebrew-tap developer checkout", result.stderr)
                if failure == "tap-wrong-root":
                    self.assertIn("tap directory must be its Git repository root", result.stderr)

    def test_partial_publication_and_invalid_assets_stop_before_tap(self):
        for failure in ("push", "publish", "actions", "checksum", "formula"):
            with self.subTest(failure=failure):
                result, calls, root, _ = self.run_release(failure=failure)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((root.parent / "homebrew-tap/Formula/reswho.rb").exists())
                self.assertIn("Release stopped during", result.stderr)
                if failure == "push":
                    self.assertNotIn(["cargo", "publish", "--locked"], calls)

    def test_invalid_version_runs_no_commands(self):
        result, calls, _, _ = self.run_release("$(touch forbidden)")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(calls, [])

    def test_actions_polling_keeps_commit_binding_and_time_budget(self):
        source = (ROOT / "scripts/release").read_text()
        polling = "run, commit = sys.argv[1:]" + source.split("run, commit = sys.argv[1:]", 1)[1].split("\nPY", 1)[0]
        result = subprocess.CompletedProcess([], 0, json.dumps({"head_sha":"releasehead","status":"in_progress","conclusion":None}))
        with mock.patch.object(sys, "argv", ["-", "4242", "releasehead"]), mock.patch("time.monotonic", side_effect=[0,0,0,1800,1800]), mock.patch("time.sleep"), mock.patch("subprocess.run", return_value=result) as request:
            with self.assertRaises(TimeoutError):
                exec(polling, {"sys":sys, "time":__import__("time"), "subprocess":subprocess, "json":json})
            self.assertEqual(request.call_args.args[0][-1], "repos/noih/reswho/actions/runs/4242")
            self.assertLessEqual(request.call_args.kwargs["timeout"], 30)
        result.stdout = json.dumps({"head_sha":"unexpected","status":"completed","conclusion":"success"})
        with mock.patch.object(sys, "argv", ["-", "4242", "releasehead"]), mock.patch("subprocess.run", return_value=result):
            with self.assertRaisesRegex(AssertionError, "commit mismatch"):
                exec(polling, {"sys":sys, "time":__import__("time"), "subprocess":subprocess, "json":json})


if __name__ == "__main__":
    unittest.main()
