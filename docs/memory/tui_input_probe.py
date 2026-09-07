"""Opt-in settings-overlay keystrokes through the real TUI PTY.

Writes are stimuli, not acknowledgments. Only native input/draw counters can
establish responsiveness. The 'o' key changes overlay visibility, not settings
or game actions (crates/tui/src/app.rs App::on_key).
"""
import json
import os
import pathlib
import threading
import time


class InputProbe:
    def __init__(self, master, run_dir, *, interval_s=1.0):
        self.fd = os.dup(master)
        self.run = pathlib.Path(run_dir)
        self.interval = interval_s
        self.stop_event = threading.Event()
        self.error = None
        self.sent = 0
        self.thread = threading.Thread(target=self._run, name='tui-input-probe')

    def start(self):
        self.thread.start()

    def close(self):
        self.stop_event.set()
        self.thread.join()

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
                       endpoint='PTY write only; not visible acknowledgment')
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
                            if os.write(self.fd, b'o') != 1:
                                raise OSError('short PTY probe write')
                            self.sent += 1
                            record('write', sequence=self.sent, before_mono_s=before,
                                   key='o', observation_boundary_seen='observe-start')
                            # No burst catch-up after a delayed wake.
                            next_send = now + self.interval
                    self.stop_event.wait(min(0.1, self.interval))
                # An odd number leaves the overlay open. Restore UI only while
                # the child is live (native end observed, not child exit).
                if not self.stop_event.is_set() and self.sent % 2:
                    if os.write(self.fd, b'o') != 1:
                        raise OSError('short PTY restore write')
                    record('restore', key='o', outside_observation=True)
                record('complete', sent=self.sent, stopped=self.stop_event.is_set())
        except Exception as error:
            self.error = str(error)
        finally:
            os.close(self.fd)
