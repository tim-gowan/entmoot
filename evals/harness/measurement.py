"""Process observations shared by campaign executors, without lifecycle ownership.

Resource counters are sampled lower bounds, not billing or exact process-tree
peaks. A caller may disable the outer timeout when a producer owns live stores.
Each invocation gets exclusive output files, including failures and retries.
"""
from __future__ import annotations

import codecs
from contextlib import nullcontext
from datetime import datetime, timezone
import json
import math
from pathlib import Path
import re
import subprocess
import tempfile
import time

import blake3
import psutil


def output_measurement(path: Path, *, tokenizer=None, token_byte_limit=16 * 1024 * 1024) -> dict:
    """Count retained bytes/decoded characters; tokenize only explicitly bounded text."""
    hasher = blake3.blake3()
    decoder = codecs.getincrementaldecoder('utf-8')(errors='replace')
    size = characters = 0
    with path.open('rb') as stream:
        while raw := stream.read(65536):
            hasher.update(raw)
            size += len(raw)
            characters += len(decoder.decode(raw))
        characters += len(decoder.decode(b'', final=True))
    tokens = None
    status = 'not_requested' if tokenizer is None else 'byte_budget_exceeded'
    if tokenizer is not None and size <= token_byte_limit:
        tokens = len(tokenizer.encode(path.read_bytes().decode('utf-8', errors='replace'), disallowed_special=()))
        status = 'counted'
    return {'bytes': size, 'characters': characters, 'character_encoding': 'utf8_replace',
            'content_digest': 'blake3:' + hasher.hexdigest(), 'tokens': tokens,
            'tokenizer': getattr(tokenizer, 'name', None), 'token_status': status}


def command(argv: list[str], cwd: Path, directory: Path, name: str,
            environment: dict, timeout: float | None, *, tokenizer=None, sampling_interval=0.05,
            stdin_path: Path | None = None) -> dict:
    """Run one command; `None` leaves producer-owned timeout/recovery in charge."""
    if timeout is not None and (not math.isfinite(timeout) or timeout <= 0):
        raise ValueError('command timeout must be positive and finite, or None')
    if not math.isfinite(sampling_interval) or sampling_interval <= 0:
        raise ValueError('resource sampling interval must be positive and finite')
    if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]*', name):
        raise ValueError('command name must be a single safe path component')
    directory.mkdir(parents=True, exist_ok=True)
    attempt = Path(tempfile.mkdtemp(prefix=name + '-', dir=directory))
    out, err = attempt / 'stdout', attempt / 'stderr'
    observed = datetime.now(timezone.utc).isoformat()
    started = time.monotonic()
    code, status, peak, peak_main = None, 'complete', None, None
    samples, cpu, io = 0, {}, {}
    limitations = {'short_lived_children_may_be_unobserved', 'rss_sum_can_count_shared_pages_more_than_once'}
    interval = sampling_interval
    launch_error = None
    with out.open('xb') as stdout, err.open('xb') as stderr, \
            (stdin_path.open('rb') if stdin_path else nullcontext(subprocess.DEVNULL)) as stdin:
        try:
            process = subprocess.Popen(argv, cwd=cwd, env=environment, stdin=stdin, stdout=stdout, stderr=stderr)
        except OSError as error:
            status = 'tool_unavailable' if isinstance(error, FileNotFoundError) else 'launch_failed'
            launch_error = str(error)
        else:
            try:
                owner = psutil.Process(process.pid)
            except psutil.NoSuchProcess:
                owner = None
            known_children = {}
            while True:
                members = []
                if owner is not None:
                    try:
                        members = [owner, *owner.children(recursive=True)]
                    except (psutil.NoSuchProcess, psutil.AccessDenied):
                        members = [owner]
                        limitations.add('process_tree_enumeration_incomplete')
                rss, observed_processes = 0, 0
                for member in members:
                    try:
                        identity = (member.pid, member.create_time())
                        if member.pid != process.pid:
                            known_children[identity] = member
                        memory = member.memory_info()
                        rss += memory.rss
                        observed_processes += 1
                        times = member.cpu_times()
                        cpu[identity] = max(cpu.get(identity, 0), times.user + times.system)
                        if member.pid == process.pid and hasattr(memory, 'peak_wset'):
                            peak_main = max(peak_main or 0, memory.peak_wset)
                        try:
                            counters = member.io_counters()
                            previous = io.get(identity, (0, 0))
                            io[identity] = (max(previous[0], counters.read_bytes), max(previous[1], counters.write_bytes))
                        except (psutil.Error, AttributeError, NotImplementedError):
                            limitations.add('io_counters_incomplete')
                    except (psutil.NoSuchProcess, psutil.AccessDenied):
                        limitations.add('process_counters_incomplete')
                if observed_processes:
                    samples += 1
                    peak = max(peak or 0, rss)
                if process.poll() is not None:
                    break
                if timeout is not None and time.monotonic() - started >= timeout:
                    status = 'timeout'
                    for child in reversed(list(known_children.values())):
                        try:
                            child.kill()
                        except psutil.NoSuchProcess:
                            pass
                        except psutil.AccessDenied:
                            limitations.add('child_termination_denied')
                    process.kill()
                    break
                remaining = interval if timeout is None else min(interval,max(0,timeout-(time.monotonic()-started)))
                try:
                    process.wait(timeout=remaining)
                except subprocess.TimeoutExpired:
                    pass
            code = process.wait()
            if code and status == 'complete':
                status = 'failed'
    elapsed = time.monotonic() - started
    result = {'status': status, 'exit_code': code, 'arguments': argv, 'cwd': str(cwd),
            'started_utc': observed, 'wall_seconds': elapsed, 'launch_error': launch_error,
            'sampled_peak_tree_rss_bytes': peak, 'os_peak_main_working_set_bytes': peak_main,
            'sampled_tree_cpu_seconds': sum(cpu.values()) if cpu else None,
            'sampled_tree_read_bytes': sum(value[0] for value in io.values()) if io else None,
            'sampled_tree_write_bytes': sum(value[1] for value in io.values()) if io else None,
            'resource_method': {'sampling_interval_seconds': interval, 'samples': samples,
                                'observed_processes': len(cpu), 'cpu': 'sum_last_observed_process_counters'},
            'stdout': str(out), 'stderr': str(err),
            'input': output_measurement(stdin_path, tokenizer=tokenizer) if stdin_path else None,
            'output': {'stdout': output_measurement(out, tokenizer=tokenizer),
                       'stderr': output_measurement(err, tokenizer=tokenizer)},
            'limitations': sorted(limitations)}
    (attempt / 'process.json').write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')
    return result


def database_bytes(locator: Path) -> int:
    """Follow a published locator; count main database and its current sidecars."""
    if locator.name.endswith('.view.json'):
        document = json.loads(locator.read_bytes())
        database = Path(document['database'])
        locator = database if database.is_absolute() else locator.parent / database
    if not locator.is_file():
        raise FileNotFoundError(locator)
    return sum(path.stat().st_size for suffix in ('', '-wal', '-shm')
               if (path := Path(str(locator) + suffix)).is_file())
