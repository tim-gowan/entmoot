"""Exercise the proxy request path without starting Docker or downloading images."""
import io
import json
import os
import subprocess
import tempfile
import unittest
from unittest.mock import Mock, patch

import docker_sidecar_proxy as proxy


class DockerScopeTests(unittest.TestCase):
    def test_selected_scope_reaches_container(self):
        with tempfile.TemporaryDirectory() as root:
            for scope in ('include', 'exclude', 'invalid', ''):
                with self.subTest(scope=scope), \
                     patch.dict(os.environ, {'ENTMOOT_GO_TESTS': scope, 'ENTMOOT_GO_SINGLE_MODULE': '1', 'ENTMOOT_GO_TAGS': 'extra'}), \
                     patch.object(proxy, '_docker_available', return_value=True), \
                     patch.object(proxy, '_ensure_image', return_value=None), \
                     patch.object(proxy.sys, 'stdin', Mock(buffer=io.BytesIO(json.dumps({'root': root}).encode()))), \
                     patch.object(proxy.sys, 'stdout', io.StringIO()), \
                     patch.object(proxy.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, b'{}', b'')) as run:
                    self.assertEqual(0, proxy.main())
                    command = run.call_args.args[0]
                    environment = [command[i + 1] for i, arg in enumerate(command) if arg == '-e']
                    self.assertIn('ENTMOOT_GO_SINGLE_MODULE=1', environment)
                    self.assertIn('ENTMOOT_GO_TAGS=extra', environment)
                    self.assertEqual([f'ENTMOOT_GO_TESTS={scope}'] if scope else [],
                                     [value for value in environment if value.startswith('ENTMOOT_GO_TESTS=')])
                    self.assertEqual('/src', json.loads(run.call_args.kwargs['input'])['root'])


if __name__ == '__main__':
    unittest.main()
