import pathlib,sys,json,hashlib
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parent/'terminal-reader'))
import pyte
p=pathlib.Path(sys.argv[1]);raw=p.read_bytes();screen=pyte.Screen(140,40);stream=pyte.Stream(screen);stream.feed(raw.decode(errors='replace'))
print('\n'.join(f'{i+1:02d}: {line}' for i,line in enumerate(screen.display)))
