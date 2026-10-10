"""Opt-in disposable V2/Zellij proof. No provider prompts are executed."""
import base64
import fcntl
import json
import os
from pathlib import Path
import pty
import secrets
import shutil
import struct
import subprocess
import tempfile
import termios
import threading
import time
import urllib.request

from live import wait_for


def main():
    binary = os.environ.get('OPENCODE_V2_TEST_BINARY') or shutil.which('opencode')
    zellij = shutil.which('zellij')
    assert binary and zellij
    with tempfile.TemporaryDirectory(prefix='tandem-v2-live-') as temporary:
        root = Path(temporary)
        workspace = root / 'workspace'
        config = root / 'config/opencode'
        workspace.mkdir()
        config.mkdir(parents=True)
        password = secrets.token_urlsafe(32)
        env = {'PATH': os.environ['PATH'], 'HOME': str(root), 'LANG': 'C.UTF-8', 'TERM': 'xterm-256color',
               'XDG_CONFIG_HOME': str(root / 'config'), 'XDG_DATA_HOME': str(root / 'data'),
               'XDG_STATE_HOME': str(root / 'state'), 'XDG_CACHE_HOME': str(root / 'cache'),
               'OPENCODE_CONFIG_DIR': str(config), 'OPENCODE_PASSWORD': password,
               'OPENCODE_DISABLE_PROJECT_CONFIG': '1', 'OPENCODE_DISABLE_MODELS_FETCH': '1',
               'OPENCODE_CONFIG_CONTENT': '{"update":"disable"}', 'OPENCODE_DISABLE_FFF': '1'}
        package = Path(os.environ.get('TANDEM_V2_PLUGIN_DIR', str(root / 'companion')))
        if 'TANDEM_V2_PLUGIN_DIR' not in os.environ:
            package.mkdir()
            shutil.copyfile(Path(__file__).resolve().parents[1] / 'bridge.mjs', package / 'bridge.mjs')
            shutil.copyfile(Path(__file__).resolve().parents[1] / 'session-input.mjs', package / 'session-input.mjs')
            (package / 'tui.js').write_text("export { default } from './bridge.mjs'\n")
        control = root / 'control'
        control.mkdir()
        route_file = root / 'route.json'
        tabs_file = root / 'tabs.json'
        tab_ids_file = root / 'tab-ids.json'
        (control / 'tui.js').write_text('''import { readFile, writeFile } from "node:fs/promises"
export default { id: "test.route", setup(ctx) {
  let previous = "", stopped = false
  const timer = setInterval(async () => {
    if (stopped) return
    try {
      await writeFile(ctx.options.tabs, JSON.stringify(ctx.ui.tabs.list().length))
      await writeFile(ctx.options.ids, JSON.stringify(ctx.ui.tabs.list().map(tab => tab.sessionID)))
      const value = await readFile(ctx.options.file, "utf8")
      if (!stopped && value !== previous) {
        previous = value
        const action = JSON.parse(value)
        if (action.close) ctx.ui.tabs.close(action.close)
        else if (action.move) ctx.ui.tabs.move(...action.move)
        else ctx.ui.router.navigate(action)
      }
    } catch {}
  }, 100)
  return () => { stopped = true; clearInterval(timer) }
} }
''')
        (config / 'cli.json').write_text(json.dumps({'plugins': [str(package), {'package': str(control), 'options': {'file': str(route_file), 'tabs': str(tabs_file), 'ids': str(tab_ids_file)}}],
                                                    'tabs': {'mode': 'on'}}))
        zconfig = root / 'zellij.kdl'
        zconfig.write_text('default_shell "/bin/sh"\npane_frames true\n')
        name = f'tandem-v2-live-{os.getpid()}'
        terminal = None
        master = None
        created = False
        with (root / 'server.log').open('w+') as log:
            server = subprocess.Popen([binary, 'serve', '--hostname', '127.0.0.1', '--port', '0'],
                                      cwd=workspace, env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=log)
            try:
                def address():
                    log.seek(0)
                    for line in log:
                        if line.startswith('server listening on '):
                            return line.strip().split('server listening on ')[1]
                url = wait_for(address)
                token = base64.b64encode(f'opencode:{password}'.encode()).decode()

                def api(path, body=None, method=None):
                    request = urllib.request.Request(url + path, method=method,
                        data=None if body is None else json.dumps(body).encode(),
                        headers={'Authorization': 'Basic ' + token, 'Content-Type': 'application/json'})
                    with urllib.request.urlopen(request, timeout=5) as response:
                        data = response.read()
                        return json.loads(data) if data else None

                first = api('/api/session', {'title': 'V2 first', 'location': {'directory': str(workspace)}})['data']['id']
                second = api('/api/session', {'title': 'V2 second', 'location': {'directory': str(workspace)}})['data']['id']
                subprocess.run([zellij, '--config', str(zconfig), 'attach', '--create-background', name],
                               cwd=workspace, env=env, capture_output=True, check=True, timeout=15)
                created = True
                master, slave = pty.openpty()
                fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 120, 0, 0))
                terminal = subprocess.Popen([zellij, 'attach', name], cwd=workspace, env=env,
                                            stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
                os.close(slave)

                def drain():
                    try:
                        while os.read(master, 65536):
                            pass
                    except OSError:
                        pass
                threading.Thread(target=drain, daemon=True).start()

                def zj(*arguments):
                    result = subprocess.run([zellij, '--session', name, *arguments], cwd=workspace, env=env,
                                            capture_output=True, text=True, timeout=10)
                    unchanged = result.returncode == 2 and (arguments[:2] in (('action', 'hide-floating-panes'), ('action', 'show-floating-panes'))
                        or arguments[:2] == ('action', 'focus-pane-id') and 'already focused' in result.stderr)
                    if result.returncode and not unchanged:
                        raise RuntimeError(result.stderr)
                    return result.stdout.strip()

                wait_for(lambda: len(zj('action', 'list-clients').splitlines()) > 1)
                pane = zj('action', 'new-pane', '--stacked', '--name', '', '--cwd', str(workspace), '--',
                          binary, str(workspace), '--server', url, '--session', first)
                pane_id = int(pane.removeprefix('terminal_'))
                presence = root / 'state/tandem/opencode'

                def records():
                    return [json.loads(path.read_text()) for path in presence.glob('*.json')]

                def receipt(id):
                    return next((record for record in records() if record['id'] == id), None)

                record = wait_for(lambda: receipt(first))
                assert record['server'] == url and record['pane_id'] == pane_id and record['zellij_session'] == name
                assert record['activity'] == 'idle'
                assert password not in json.dumps(record)
                print('PASS: V2 loads the companion package and publishes exact session/pane/server identity')
                route_file.write_text(json.dumps({'type': 'session', 'sessionID': second}))
                wait_for(lambda: receipt(second))
                assert not receipt(first)
                wait_for(lambda: any(tab['id'] == first for tab in receipt(second).get('tabs', [])))
                print('PASS: background session tabs remain attached in the companion receipt')
                def native_order(record):
                    tabs = record.get('tabs', []) + ([record] if record['id'] else [])
                    return [tab['id'] for tab in sorted(tabs, key=lambda tab: tab['tab_index'])]
                before_tabs = len(native_order(receipt(second)))
                before_panes = json.loads(zj('action', 'list-panes', '--all', '--json'))
                control_receipt = receipt(second)['tab_control']
                def tab_action(path, body):
                    request = urllib.request.Request(control_receipt['server'] + path,
                        data=json.dumps(body).encode(),
                        headers={'Authorization': 'Bearer ' + control_receipt['token'], 'Content-Type': 'application/json'})
                    try:
                        with urllib.request.urlopen(request, timeout=15) as response:
                            return json.load(response)['id']
                    except urllib.error.HTTPError as error:
                        raise AssertionError(f'{path}: {error.code}: {error.read().decode()}') from error

                instructions = 'Services are starting automatically; explore available code.'
                assert tab_action('/tabs', {'directory': str(workspace), 'instructions': instructions}) == first
                wait_for(lambda: receipt(first))
                entries_path = f'/api/experimental/session/{first}/instructions/entries'
                assert any(entry['key'] == 'tandem.services' and entry['value'] == instructions
                           for entry in api(entries_path)['data'])
                instructions = "Services won't start automatically; ask if needed."
                assert tab_action('/tabs', {'directory': str(workspace), 'instructions': instructions}) == first
                assert any(entry['key'] == 'tandem.services' and entry['value'] == instructions
                           for entry in api(entries_path)['data'])
                assert len(native_order(receipt(first))) == before_tabs
                assert len(api('/api/session?limit=20')['data']) == 2
                print('PASS: empty-tab reuse replaces durable service instructions without creating a session')
                route_file.write_text(json.dumps({'close': second}))
                wait_for(lambda: second not in native_order(receipt(first)))
                route_file.write_text(json.dumps({'close': first}))
                wait_for(lambda: receipt('') and not receipt('').get('tabs'))
                new_id = tab_action('/tabs', {'directory': str(workspace), 'instructions': instructions})
                assert any(entry['key'] == 'tandem.services' and entry['value'] == instructions
                           for entry in api(f'/api/experimental/session/{new_id}/instructions/entries')['data'])
                new_record = wait_for(lambda: receipt(new_id))
                assert new_record['pid'] == record['pid'] and new_record['pane_id'] == pane_id
                assert new_id not in (first, second)
                assert len(native_order(new_record)) == 1
                assert len(json.loads(zj('action', 'list-panes', '--all', '--json'))) == len(before_panes)
                print('PASS: companion creates and focuses a native session tab in the same client/pane')
                route_file.write_text(json.dumps({'type': 'session', 'sessionID': first}))
                wait_for(lambda: receipt(first))
                route_file.write_text(json.dumps({'type': 'session', 'sessionID': second}))
                wait_for(lambda: receipt(second) and first in native_order(receipt(second)))
                assert tab_action('/tabs/focus', {'sessionID': first}) == first
                wait_for(lambda: receipt(first))
                assert tab_action('/tabs/focus', {'sessionID': second}) == second
                wait_for(lambda: receipt(second))
                print('PASS: companion navigation selects an existing background tab in the same pane')
                expected = json.loads(tab_ids_file.read_text())
                expected.remove(first)
                expected.append(first)
                started = time.monotonic()
                route_file.write_text(json.dumps({'move': [first, len(expected) - 1]}))
                wait_for(lambda: native_order(receipt(second)) == expected, timeout=2)
                assert time.monotonic() - started < 1
                print('PASS: native tab reordering publishes matching positions in under one second')

                request = urllib.request.Request(url + '/api/event', headers={'Authorization': 'Basic ' + token})
                with urllib.request.urlopen(request, timeout=5) as stream:
                    def event():
                        while True:
                            line = stream.readline()
                            assert line, 'Live event stream closed'
                            if line.startswith(b'data:'):
                                value = json.loads(line.removeprefix(b'data:').strip())
                                return json.loads(value) if isinstance(value, str) else value
                    wait_for(lambda: event().get('type') == 'server.connected')
                    started = time.monotonic()
                    api(f'/api/session/{second}', {'title': 'V2 renamed'}, method='PATCH')
                    wait_for(lambda: second in json.dumps(event()))
                    wait_for(lambda: receipt(second)['title'] == 'V2 renamed', timeout=2)
                    assert time.monotonic() - started < 1
                print('PASS: live server events and reactive title updates arrive in under one second')

                def target():
                    return next(item for item in json.loads(zj('action', 'list-panes', '--all', '--json')) if not item['is_plugin'] and item['id'] == pane_id)

                wait_for(lambda: target()['title'] == 'OC | V2 renamed')
                print('PASS: native route switching and title updates stay attached to the same pane')
                sibling = zj('action', 'new-pane', '--stacked', '--', 'sleep', '60')
                floating = zj('action', 'new-pane', '--floating', '--', 'sleep', '60')
                for _ in range(2):
                    zj('action', 'go-to-tab-by-id', str(target()['tab_id']))
                    zj('action', 'hide-floating-panes', '--tab-id', str(target()['tab_id']))
                    zj('action', 'focus-pane-id', pane)
                actual = target()
                assert actual['is_focused'] and not actual['is_suppressed'] and actual['pane_content_rows'] > 1
                print('PASS: repeated focus expands the exact stacked pane from a floating pane')
                route_file.write_text(json.dumps({'type': 'home'}))
                home = wait_for(lambda: receipt(''))
                assert {tab['id'] for tab in home['tabs']} >= {first, second, new_id}
                print('PASS: the home route keeps every open session tab attached')
                assert tab_action('/tabs/close', {'sessionID': first}) == first
                wait_for(lambda: not any(tab['id'] == first for record in records() for tab in record.get('tabs', [])))
                assert receipt('')
                assert {tab['id'] for tab in receipt('')['tabs']} >= {second, new_id}
                assert target()['id'] == pane_id
                assert api(f'/api/session/{first}')['data']['id'] == first
                assert tab_action('/tabs/focus', {'sessionID': second}) == second
                wait_for(lambda: receipt(second))
                assert tab_action('/tabs/close', {'sessionID': second}) == second
                wait_for(lambda: receipt(new_id) and not any(tab['id'] == second for tab in receipt(new_id).get('tabs', [])))
                assert target()['id'] == pane_id
                assert tab_action('/tabs/close', {'sessionID': new_id}) == new_id
                wait_for(lambda: receipt('') and not receipt('').get('tabs'))
                assert target()['id'] == pane_id
                print('PASS: companion closes background, active and last tabs while preserving siblings, history and the client pane')
                zj('action', 'close-pane', '--pane-id', floating)
                zj('action', 'close-pane', '--pane-id', sibling)
                zj('action', 'close-pane', '--pane-id', pane)
                wait_for(lambda: not records() or all(not Path(f'/proc/{record["pid"]}').exists() for record in records()), timeout=15)
                assert api('/api/session/active')['data'] == {}
                print('PASS: closing the client removes presence; no model run occurred')
                route_file.unlink()
                for model, variant, expected in [
                    ('', '', None),
                    ('openai/gpt-5.2', '', {'providerID': 'openai', 'id': 'gpt-5.2', 'variant': 'default'}),
                    ('openai/gpt-5.2', 'high', {'providerID': 'openai', 'id': 'gpt-5.2', 'variant': 'high'}),
                ]:
                    guided_pane = zj('action', 'new-pane', '--stacked', '--name', '', '--cwd', str(workspace), '--',
                                    'env', 'TANDEM_INITIAL_PROMPT=', f'TANDEM_SESSION_INSTRUCTIONS={instructions}',
                                    f'TANDEM_SESSION_MODEL={model}', f'TANDEM_SESSION_VARIANT={variant}',
                                    binary, str(workspace), '--server', url)
                    guided_pane_id = int(guided_pane.removeprefix('terminal_'))
                    try:
                        guided = wait_for(lambda: next((record for record in records()
                            if record['pane_id'] == guided_pane_id and record['id']
                            and 'tab_index' in record), None))
                    except AssertionError:
                        print(f'Fresh-client setup failed: model={model!r}, variant={variant!r}')
                        print(zj('action', 'dump-screen', '--pane-id', guided_pane, '--full'))
                        raise
                    assert native_order(guided) == [guided['id']], guided
                    assert any(entry['key'] == 'tandem.services' and entry['value'] == instructions
                               for entry in api(f'/api/experimental/session/{guided["id"]}/instructions/entries')['data'])
                    actual = api(f'/api/session/{guided["id"]}')['data'].get('model')
                    assert actual == expected, {'requested': model, 'variant': variant, 'expected': expected, 'actual': actual}
                    assert api('/api/session/active')['data'] == {}
                    control_receipt = guided['tab_control']
                    assert tab_action('/tabs/close', {'sessionID': guided['id']}) == guided['id']
                    wait_for(lambda: receipt('') and not receipt('').get('tabs'))
                    zj('action', 'close-pane', '--pane-id', guided_pane)
                    wait_for(lambda: not records() or all(not Path(f'/proc/{record["pid"]}').exists() for record in records()), timeout=15)
                print('PASS: fresh clients preserve model/variant and instructions, while closed tabs stay closed across client restarts')
            finally:
                if created:
                    subprocess.run([zellij, 'kill-session', name], env=env, capture_output=True, timeout=10)
                if terminal:
                    terminal.terminate()
                    terminal.wait(timeout=5)
                if master is not None:
                    os.close(master)
                server.terminate()
                try:
                    server.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait()


if __name__ == '__main__':
    main()
