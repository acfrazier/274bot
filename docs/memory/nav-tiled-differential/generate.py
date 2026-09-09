"""Deterministic bounded wire corpus, frozen before either source arm runs.
This writes bytes, not expected outcomes. Error truth is the original decoder.
"""
import hashlib
import json
import struct
from pathlib import Path

U = lambda x: struct.pack('<I', x)
I = lambda x: struct.pack('<i', x)
def text(s):
    b = s.encode() if isinstance(s,str) else s
    return U(len(b))+b

def header(w=3,h=3,origin=(100,200,2),magic=b'274V',version=8):
    return magic+bytes([version])+b''.join(I(v) for v in origin)+U(w)+U(h)

def edge(kind=0, at=(101,201,0), to=(101,201,1), req=False, ident=42, direction=0):
    b = bytes([kind])+b''.join(I(v) for v in (*at,*to,ident,1,1))+bytes([direction])+I(ident+1)
    pairs = lambda v: U(len(v))+b''.join(I(a)+I(c) for a,c in v)
    return b+pairs([(6,25),(2,3)] if req else [])+pairs([(995,10)] if req else [])+(U(2)+text('Rune Mysteries')+text('é') if req else U(0))+pairs([(150,160)] if req else [])+(U(2)+I(1712)+I(946) if req else U(0))

def bank(kind=0, choose=1):
    b=text('Bänk')+I(100)+I(200)+I(0)+bytes([kind])
    return b+(I(2) if kind==0 else text('Teller')+I(1)+bytes([choose])+(text('Yes') if choose else b''))

def pack(w=3,h=3,origin=(100,200,2),pairs=None,edges=(),banks=(),tail=b''):
    n=w*h*4
    if pairs is None: pairs=[(0,False)]*n
    assert len(pairs)==n
    words=[0]*((n+63)//64)
    for i,(_,v) in enumerate(pairs):
        if v: words[i//64]|=1<<(i%64)
    if n%64: words[-1]|=((1<<64)-1)^((1<<(n%64))-1)
    return header(w,h,origin)+bytes(f for f,_ in pairs)+b''.join(struct.pack('<Q',v) for v in words)+U(len(edges))+b''.join(edges)+U(len(banks))+b''.join(banks)+tail

def generate(run):
    folder=run/'inputs'; folder.mkdir()
    (run/'fixed.tsv').write_text(''.join('100 200 0 102 202 3 %d %d %d %d\n'%(bits,sm,radius,model)
        for bits in (0,1,4,8,15) for sm in (0,1,3) for radius in (0,1,2) for model in (0,1)))
    corpus=[]
    def add(name,b,kind='pack',routes='none',coverage='wire'):
        assert len(b)<=1024**2
        path=folder/(name+'.bin'); path.write_bytes(b)
        corpus.append(dict(path=str(path.relative_to(run)),sha256=hashlib.sha256(b).hexdigest(),bytes=len(b),kind=kind,routes=routes,coverage=coverage))
    # Interleaved teleport order, duplicate at indices and ordered requirements.
    edges=[edge(4,ident=90),edge(2,req=True),edge(0,direction=2),edge(4,ident=91),edge(1,ident=43)]
    for k in (3,5,6,7,8): edges.append(edge(k,ident=100+k))
    full=pack(edges=edges,banks=[bank(),bank(1),bank(1,0)])
    add('ordered',full)
    add('trailing',full+bytes(range(32)))
    for i in range(len(full)): add('trunc-%04d'%i,full[:i],coverage='every-byte-truncation')
    for v in range(256):
        if v!=8: add('version-%03d'%v,full[:4]+bytes([v])+full[5:])
    for w,h in [(0,1),(1,0),(16385,1),(1,16385),(0xffffffff,0xffffffff)]:
        add('dimension-%s-%s'%(w,h),header(w,h))
    for magic in (b'274N',b'274F',b'xxxx'):
        add('magic-'+magic.decode(),magic+full[4:])
    # Mutate only known one-byte tags/string payloads, never lengths into bombs.
    simple=pack(edges=[edge(req=True)],banks=[bank(1)])
    edge_at=25+36+8+4
    for tag in range(9,256):
        b=bytearray(simple); b[edge_at]=tag; add('kind-%03d'%tag,bytes(b))
    for tag in range(5,256):
        b=bytearray(simple); b[edge_at+37]=tag; add('dir-%03d'%tag,bytes(b))
    for token in ('Rune Mysteries','é','Bänk','Teller','Yes'):
        b=bytearray(simple); b[b.index(token.encode())]=255; add('utf8-'+token.encode().hex(),bytes(b))
    bank_tag=simple.index('Bänk'.encode())+len('Bänk'.encode())+12
    for tag in range(2,256):
        b=bytearray(simple); b[bank_tag]=tag; add('bank-tag-%03d'%tag,bytes(b))
    for tag in (2,255):
        b=bytearray(simple); b[b.index(b'Teller')+len(b'Teller')+4]=tag; add('presence-%03d'%tag,bytes(b))
    # Bounded oversized string lengths/requirement counts; error expectations are not authored.
    for offset in (edge_at+42,edge_at+62):
        b=bytearray(simple); b[offset:offset+4]=U(100); add('count-%d'%offset,bytes(b))
    side=header(magic=b'274F',version=1)+b''.join(U(i*12345) for i in range(36))
    for i in range(len(side)+1): add('side-%03d'%i,side[:i],kind='sidecar')
    for v in (0,2,255): add('side-version-%d'%v,side[:4]+bytes([v])+side[5:],kind='sidecar')
    add('side-extra-word',side+U(0xffffffff),kind='sidecar')
    # Actual load_pack uses original BadMagic-only grid fallback (second file read).
    grid=header(magic=b'274N',version=1)+bytes([1,0,1,1,0,1,1,1,1])+U(1)+b''.join(I(v) for v in (101,201,0,50,100,201,0,102,201,0))
    for i in range(len(grid)+1): add('grid-%03d'%i,grid[:i],coverage='grid-fallback')
    # Exhaust all ordered pairs in each 3x3x4 map. Origins hit wilderness/essence
    # regions as well as ordinary negative/nonzero geometry; sealed targets.
    for name,origin in [('open',(100,200,2)),('wild',(3000,3520,0)),('essence',(2884,4849,0)),('negative',(-3,-3,3))]:
        x,z,_=origin
        es=[edge(0,(x+1,z+1,0),(x+1,z+1,1),True),edge(2,(x,z,1),(x+2,z+2,2)),edge(4,(0,0,0),(x+2,z+2,3)),edge(1,(x+2,z,2),(x,z+2,3))]
        add('routes-'+name,pack(origin=origin,edges=es,banks=[bank(),bank(1)]),routes='all',coverage='all-pairs-options-models')
    pairs=[(0,False)]*36
    for p in range(4): pairs[p*9+4]=(255,True)
    add('routes-sealed',pack(pairs=pairs),routes='all',coverage='all-pairs-options-models')
    for w in (1,31,32,33,63,64,65):
        for h in (1,31,32,33,63,64,65):
            pairs=[((i*73+19)&255,bool((i*17+3)&256)) for i in range(w*h*4)]
            add('geometry-%d-%d'%(w,h),pack(w,h,(-7,13,3),pairs),coverage='dimension-cross-product')
    for value in range(512):
        add('uniform-%03d'%value,pack(1,1,pairs=[(value&255,bool(value&256))]*4),coverage='all-uniform-pairs')
    # Local masks: each of 9 positions takes every exact nine-bit value;
    # then every joint 3x3 blocked occupancy. Identical neighborhoods at
    # x31 and x63 cover 31/32 and 63/64, independently on all four planes.
    for family,total in [('pair',9*512),('occupancy',512)]:
        for case in range(total):
            pairs=[(0,False)]*(65*3*4)
            for plane in range(4):
                for center in (31,63):
                    for pos in range(9):
                        value=(case%512 if pos==case//512 else 0) if family=='pair' else (256 if case&(1<<pos) else 0)
                        idx=plane*195+(pos//3)*65+center-1+pos%3
                        pairs[idx]=(value&255,bool(value&256))
            add('corner-%s-%04d'%(family,case),pack(65,3,pairs=pairs),kind='corner',coverage='local-3x3-'+family)
            transposed=[pairs[p*195+z*65+x] for p in range(4) for x in range(65) for z in range(3)]
            add('corner-z-%s-%04d'%(family,case),pack(3,65,pairs=transposed),kind='corner',coverage='local-3x3-z-'+family)
    (run/'corpus.json').write_text(json.dumps(corpus,indent=2,sort_keys=True)+'\n')
    (run/'coverage.json').write_text(json.dumps({key:sum(e['coverage']==key for e in corpus) for key in sorted({e['coverage'] for e in corpus})},indent=2)+'\n')
