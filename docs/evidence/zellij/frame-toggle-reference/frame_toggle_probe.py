from pathlib import Path
import tempfile
from pane_workflow_differential import run_side, write_spawn_shell
ROOT = Path('/tmp/ekko-reference-frame-toggle')
ROOT.mkdir(parents=True, exist_ok=True)
reference = Path('/home/y0usaf/dev/maintaining/ekko_v2/tests/zellij/reference').resolve()
output = ROOT / '80x24'
output.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory(prefix='frame-toggle-probe-') as td:
    root = Path(td)
    shell = write_spawn_shell(root)
    stages = [('startup', b''), ('dismiss-release-notes', b'\x1b'),
              ('pane-enter', b'\x10'), ('frames-off', b'z'),
              ('pane-enter-off', b'\x10'), ('frames-on', b'z')]
    result = run_side('zellij', '/nix/store/wxjfyl2ksqkh263zwikp2igqh2a23y8r-zellij-0.43.1/bin/zellij',
                      Path('/home/y0usaf/dev/maintaining/ekko_v2/examples/profiles/zellij.lisp'),
                      reference, root, output, shell, stages, 80, 24)
    import json
    (ROOT/'stages.json').write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps({'stages':[x['name'] for x in result], 'output':str(ROOT)}))
