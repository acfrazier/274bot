"""Opt-in settings-overlay keystrokes through the real TUI terminal transport.

Writes are stimuli, not acknowledgments. Only native input/draw counters can
establish responsiveness. The 'o' key changes overlay visibility, not settings
or game actions (crates/tui/src/app.rs App::on_key).

``master`` may be:
  - an int PTY master fd (Unix) — duplicated and written with os.write
  - an object with ``write(bytes) -> int`` (Windows ConPTY input writer)
"""
import json
import os
import pathlib
import threading
import time


class InputProbe:
    def __init__(self, master, run_dir, *, interval_s=1.0):
        self.run = pathlib.Path(run_dir)
        self.interval = interval_s
        self.stop_event = threading.Event()
        self.error = None
        self.sent = 0
        self.thread = threading.Thread(target=self._run, name='tui-input-probe')
        self._transport = None
        self.fd = None
        if isinstance(master, int):
            self.fd = os.dup(master)
        elif hasattr(master, 'write'):
            self._transport = master
        else:
            raise TypeError('InputProbe master must be a PTY fd or a write() transport')

    def start(self):
        self.thread.start()

    def close(self):
        self.stop_event.set()
        self.thread.join()

    def _write_bytes(self, data: bytes) -> int:
        if self._transport is not None:
            n = self._transport.write(data)
            return int(n)
        return os.write(self.fd, data)

    def _phase(self):
        path = self.run / 'samples.qualification.jsonl'
        if not path.exists():
            return None
        # A producer may be in the middle of its final line.
        data = path.read_bytes()
        lines = data.split(b'\n')[:-1]
        rows = [json.loads(line) for line in lines if line.strip()]
        phases = [row.get('phase') for row in rows]
        if phases not in ([], ['observe-start'], ['observe-start', 'observe-end']):
            raise ValueError('invalid native observation boundary sequence')
        return phases[-1] if phases else None

    def _run(self):
        try:
            with (self.run / 'input-probes.jsonl').open('x') as log:
                def record(kind, **fields):
                    log.write(json.dumps(dict(kind=kind, mono_s=time.monotonic(),
                                              wall_s=time.time(), **fields)) + '\n')
                    log.flush()
                record('configuration', key='o', interval_s=self.interval,
                       endpoint='terminal write only; not visible acknowledgment')
                next_send = None
                while not self.stop_event.is_set():
                    phase = self._phase()
                    if phase == 'observe-end':
                        break
                    if phase == 'observe-start':
                        now = time.monotonic()
                        if next_send is None:
                            next_send = now
                        if now >= next_send:
                            before = time.monotonic()
                            if self._write_bytes(b'o') != 1:
                                raise OSError('short terminal probe write')
                            self.sent += 1
                            record('write', sequence=self.sent, before_mono_s=before,
                                   key='o', observation_boundary_seen='observe-start')
                            # No burst catch-up after a delayed wake.
                            next_send = now + self.interval
                    self.stop_event.wait(min(0.1, self.interval))
                # An odd number leaves the overlay open. Restore UI only while
                # the child is live (native end observed, not child exit).
                if not self.stop_event.is_set() and self.sent % 2:
                    if self._write_bytes(b'o') != 1:
                        raise OSError('short terminal restore write')
                    record('restore', key='o', outside_observation=True)
                record('complete', sent=self.sent, stopped=self.stop_event.is_set())
        except Exception as error:
            self.error = str(error)
        finally:
            if self.fd is not None:
                try:
                    os.close(self.fd)
                except OSError:
                    pass
                # Leave closed fd int so callers can observe EBADF via fstat.
