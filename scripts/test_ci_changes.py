#!/usr/bin/env python3
"""Regression tests for CI job selection, git comparisons and final status gating."""

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import textwrap
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parent


def load_script(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), SCRIPTS / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ci = load_script('ci-changes')
docs = load_script('check-docs')


class SelectionTests(unittest.TestCase):
    def selected(self, *paths):
        return {name for name, value in ci.select(paths).items() if value}

    def test_docs_do_not_start_native_builds(self):
        self.assertEqual(self.selected('README.md', 'README_zh-TW.md', 'docs/TESTING.md'), set())
        self.assertEqual(self.selected('docs/USER_GUIDE_zh-TW.adoc'), {'guide'})
        self.assertEqual(self.selected('images/editor.png'), {'guide'})
        self.assertEqual(self.selected('docs/guide-zh-TW-theme.yml'), {'guide'})

    def test_rust_changes_only_need_linux_checks(self):
        self.assertEqual(self.selected('src/publish.rs', 'docs/TESTING.md'), {'rust'})
        self.assertEqual(self.selected('src/bench.rs'), {'rust', 'bench'})
        self.assertEqual(self.selected('Cargo.lock'), {'rust', 'deny'})
        self.assertEqual(self.selected('Cargo.toml'), {'rust', 'deny'})
        self.assertEqual(self.selected('deny.toml'), {'deny'})

    def test_wordpress_changes_do_not_build_the_app_except_shared_test_inputs(self):
        self.assertEqual(self.selected('wordpress/picasso-places/inc/places.php'), {'wordpress'})
        self.assertEqual(self.selected('scripts/build-theme.py'), {'wordpress'})
        self.assertEqual(self.selected('wordpress/voyagelight/assets/css/theme.css'), {'wordpress', 'rust'})
        self.assertEqual(self.selected('wordpress/tests/geo-cases.json'), {'wordpress'})
        self.assertEqual(self.selected('wordpress/README.zh-TW.md'), set())

    def test_packaging_changes_select_only_their_platforms(self):
        self.assertEqual(self.selected('scripts/package-appimage.sh', 'build/AppRun'), {'appimage'})
        self.assertEqual(self.selected('docker/appimage/Dockerfile'), {'appimage'})
        self.assertEqual(self.selected('build/picasso.desktop'), {'linux', 'appimage'})
        self.assertEqual(self.selected('build/picasso.png'), {'linux', 'appimage', 'macos'})
        self.assertEqual(self.selected('build/picasso.ico'), {'rust', 'windows'})
        self.assertEqual(self.selected('scripts/self-signed-identity.sh'), {'macos'})
        self.assertEqual(self.selected('scripts/third-party-notices.py'), {'windows', 'linux', 'appimage', 'macos'})

    def test_guide_tool_changes_only_render_guides(self):
        for path in ['scripts/guide-fonts.sh', 'scripts/docs-pdf.sh', 'docker/guide-pdf/Dockerfile']:
            self.assertEqual(self.selected(path), {'guide'})

    def test_build_policy_and_unknown_inputs_require_full_validation(self):
        for path in ['.github/workflows/ci.yml', 'scripts/ci-changes.py', 'scripts/test_ci_changes.py',
                     'Makefile', 'build.rs', 'rust-toolchain.toml', '.cargo/config.toml', 'new-build-input.json']:
            self.assertTrue(all(ci.select([path]).values()), path)
        self.assertTrue(all(ci.select([], full=True).values()))

    def test_large_changes_do_not_truncate_rust_files(self):
        paths = [f'docs/file-{n}.md' for n in range(400)] + ['src/publish.rs']
        self.assertEqual(self.selected(*paths), {'rust'})

    def test_missing_comparison_falls_back_to_all_checks(self):
        with tempfile.TemporaryDirectory() as directory:
            event = Path(directory) / 'event.json'
            output = Path(directory) / 'output'
            event.write_text(json.dumps({'before': '0' * 40, 'after': 'a' * 40}))
            with patch.dict(os.environ, {'GITHUB_EVENT_NAME': 'push', 'GITHUB_EVENT_PATH': str(event),
                                         'GITHUB_OUTPUT': str(output)}, clear=True):
                with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                    ci.main()
            self.assertEqual(set(output.read_text().splitlines()), {f'{name}=true' for name in ('full', *ci.JOBS)})

    def test_tags_and_manual_runs_do_not_need_comparison_history(self):
        for event, ref in [('push', 'refs/tags/v0.9.6'), ('workflow_dispatch', 'refs/heads/main')]:
            output = io.StringIO()
            with patch.dict(os.environ, {'GITHUB_EVENT_NAME': event, 'GITHUB_REF': ref}, clear=True):
                with contextlib.redirect_stdout(output):
                    ci.main()
            self.assertEqual(set(output.getvalue().splitlines()), {f'{name}=true' for name in ('full', *ci.JOBS)})


class GitComparisonTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.git('init', '-q', '-b', 'main')
        self.write('README.md', 'Initial\n')
        self.commit()
        self.initial = self.sha()

    def git(self, *args):
        return subprocess.check_output(['git', *args], cwd=self.root, stderr=subprocess.PIPE)

    def write(self, name, text):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def commit(self):
        self.git('add', '.')
        self.git('-c', 'user.name=CI', '-c', 'user.email=ci@example.test', '-c', 'commit.gpgsign=false', 'commit', '-qm', 'Test')

    def sha(self):
        return self.git('rev-parse', 'HEAD').decode().strip()

    def test_push_includes_all_commits_in_the_push(self):
        self.write('src/new.rs', 'fn main() {}\n')
        self.commit()
        self.write('docs/new.md', 'Docs\n')
        self.commit()
        with patch.object(ci, 'git', self.git):
            paths = ci.changed_paths('push', {'before': self.initial}, self.sha())
        self.assertEqual(set(paths), {'src/new.rs', 'docs/new.md'})
        self.assertTrue(ci.select(paths)['rust'])

    def test_pr_excludes_changes_that_only_happened_on_the_base_branch(self):
        self.git('checkout', '-qb', 'feature')
        self.write('README.md', 'Feature documentation\n')
        self.commit()
        head = self.sha()
        self.git('checkout', '-q', 'main')
        self.write('src/base-only.rs', 'fn main() {}\n')
        self.commit()
        with patch.object(ci, 'git', self.git):
            paths = ci.changed_paths('pull_request', {'pull_request': {'base': {'sha': self.sha()}}}, head)
        self.assertEqual(paths, ['README.md'])
        self.assertFalse(ci.select(paths)['rust'])

    def test_rename_includes_the_removed_rust_path_and_odd_filenames(self):
        self.write('src/old.rs', 'fn main() {}\n')
        self.commit()
        base = self.sha()
        self.root.joinpath('docs').mkdir()
        self.git('mv', 'src/old.rs', 'docs/renamed.md')
        if os.name != 'nt':
            self.write('docs/a\nfile.md', 'An unusual filename\n')
        self.commit()
        with patch.object(ci, 'git', self.git):
            paths = ci.changed_paths('push', {'before': base}, self.sha())
        expected = {'src/old.rs', 'docs/renamed.md'}
        if os.name != 'nt':
            expected.add('docs/a\nfile.md')
        self.assertEqual(set(paths), expected)
        self.assertTrue(ci.select(paths)['rust'])


class ResultGateTests(unittest.TestCase):
    def gate(self, paths, overrides=None, full=False):
        plan = ci.select(paths, full=full)
        names = ['changelog', 'test', 'deny', 'windows', 'linux', 'appimage', 'appimage-test', 'macos', 'guide']
        results = {name: {'result': 'skipped'} for name in names}
        results['changelog'] = {'result': 'success', 'outputs': {key: str(value).lower() for key, value in plan.items()}}
        for name in names[1:]:
            needed = (plan['rust'] and not plan['linux'] if name == 'test'
                      else plan['appimage'] if name == 'appimage-test' else plan[name])
            if needed:
                results[name]['result'] = 'success'
        for name, result in (overrides or {}).items():
            results[name]['result'] = result
        workflow = (SCRIPTS.parent / '.github/workflows/ci.yml').read_text()
        step = workflow.split('- name: Check selected job results', 1)[1]
        code = re.search(r"python3 - <<'PYTHON'\n(.*?)\n          PYTHON", step, re.S).group(1)
        return subprocess.run([os.sys.executable, '-c', textwrap.dedent(code)],
                              env={**os.environ, 'RESULTS': json.dumps(results)}, capture_output=True)

    def test_intentionally_skipped_jobs_pass(self):
        for paths in [[], ['README.md'], ['src/publish.rs'], ['build/AppRun']]:
            self.assertEqual(self.gate(paths).returncode, 0, paths)
        self.assertEqual(self.gate([], full=True).returncode, 0)

    def test_failed_cancelled_and_unexpectedly_skipped_checks_fail(self):
        for status in ['failure', 'cancelled', 'skipped']:
            self.assertNotEqual(self.gate(['src/publish.rs'], {'test': status}).returncode, 0)
            self.assertNotEqual(self.gate([], {'windows': status}, full=True).returncode, 0)
        self.assertNotEqual(self.gate([], {'changelog': 'failure'}).returncode, 0)
        self.assertNotEqual(self.gate(['build/AppRun'], {'appimage-test': 'skipped'}).returncode, 0)


class TranslationDocsTests(unittest.TestCase):
    def test_translated_text_can_change_but_commands_and_structure_must_match(self):
        en = '# Guide\n\n- Open `settings.json`.\n'
        zh = '# 指南\n\n- 開啟 `settings.json`。\n'
        self.assertEqual(docs.outline(en), docs.outline(zh))
        self.assertNotEqual(docs.outline(en), docs.outline(zh.replace('settings.json', 'setting.json')))
        self.assertNotEqual(docs.outline(en), docs.outline(zh.replace('# 指南', '## 指南')))

    def test_traditional_chinese_wording_stays_checked_without_rust(self):
        rules = docs.read('src/i18n.rs')
        documents = {
            'README.md': '# Guide\n',
            'README_zh-TW.md': '# 指南\n視頻设\n',
            'docs/USER_GUIDE.adoc': '== Guide\n',
            'docs/USER_GUIDE_zh-TW.adoc': '== 指南\n',
            'src/i18n.rs': rules,
        }
        with patch.object(docs, 'read', documents.__getitem__):
            problems = docs.bilingual_problems()
        self.assertTrue(any('視頻 should be 影片' in problem for problem in problems))
        self.assertTrue(any('Simplified Chinese' in problem for problem in problems))

    def test_asciidoc_keyboard_shortcuts_are_compared(self):
        en = '[#start]\n== Start\nPress kbd:[Ctrl+F].\n'
        zh = '[#start]\n== 開始\n按 kbd:[Ctrl+F]。\n'
        self.assertEqual(docs.outline(en, True), docs.outline(zh, True))
        self.assertNotEqual(docs.outline(en, True), docs.outline(zh.replace('Ctrl+F', 'Ctrl+G'), True))


if __name__ == '__main__':
    unittest.main()
