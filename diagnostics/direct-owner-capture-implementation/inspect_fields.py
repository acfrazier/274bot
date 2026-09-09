from pathlib import Path
import re
for p in ['crates/api/src/snapshot_owner_capture.rs','crates/host/src/owner_capture.rs','vendor/fr-client-rust/crates/client/src/core/world_owner_capture.rs']:
    t=Path(p).read_text()
    print(p, sorted(set(re.findall(r'"([a-z][a-z_.]+)"',t))))
