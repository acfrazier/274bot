from pathlib import Path
import json
import sys
sys.path.insert(0, str(Path(__file__).parents[3]))
from frozen_support import bounded

result = bounded(
    ['python3', '-m', 'unittest', '-v', 'test_sharded', 'test_sharded_guards'],
    Path('diagnostics/nav-stage-a-native-preparation/scheduler-fixture-correction/bounded'),
    'scheduler-full', wall=360, cpu=300, rss=512*1024**2,
    address=4*1024**3, output=1024**2, cwd=Path('.'))
print(json.dumps(result, sort_keys=True))
