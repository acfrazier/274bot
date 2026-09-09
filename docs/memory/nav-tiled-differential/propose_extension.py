#!/usr/bin/env python3
"""Freeze static proposed selectors from approved sources; NEVER open a navpack.
Writes a new owned directory. No subprocess, network, foreign runtime or account.
"""
import argparse
import hashlib
import json
from pathlib import Path
import harness as h

SOURCES = {
    'e2e/nav-tele-smoke.ts': '1aebcdabc3d71ece90860a56c9254920a89ed4613b475cba1143ab05a21ca641',
    'e2e/nav-stress-live.ts': 'b3fbe043e5354929a43b675244de9aade975262e3df12e4135f16c64360c2382',
    'tools/nav/transport-heavy-routes.ts': '085316415c11eefad6d6605eb04d5029b3544e798cfc1d50748fc4daf6b1fe35',
    'tools/nav/mainland-routes.json': 'a63752fe5713a7fe602e7f544d9eff5ff58e3ac9888e0e1cd07d0dc2f54c6c95',
}

def propose(source, dest):
    if h.HERE not in dest.parents: raise ValueError('owned tool output only')
    for name,digest in SOURCES.items(): h.admit(source/name,digest,1024**2)
    invpath=h.ROOT/'diagnostics/native-nav-differential-preparation/results-760d3ac/metadata/root-real-selector-inventory.json'
    metadata=json.loads(invpath.read_text());edges={e['index']:e for e in metadata['edges']}
    anchors={}
    lines=invpath.read_text().splitlines()
    for index,kind,items,skills,target in [
        (2143,4,[[554,1],[556,3],[563,1]],[[6,25]],[3213,3424,0]),
        (2166,4,[[1712,1]],[],[3087,3496,0]),
        (2167,4,[[1712,1]],[],[2918,3176,0]),
        (2089,3,[[995,30]],[],[2956,3143,1]),
        (2095,3,[[995,30]],[],[2775,3234,1]),
    ]:
        e=edges[index]
        assert e['kind']==kind and e['items']==items and e['skills']==skills and e['to']==target
        assert e['quests']==e['varps']==e['worn']==[]
        anchors[str(index)]=dict(edge=e,path=str(invpath.relative_to(h.ROOT)),sha256=h.sha(invpath),
                                line=next(i for i,line in enumerate(lines,1) if line.strip()==f'"index": {index},'))
    def origin(name,start,end):
        return dict(path=name,sha256=SOURCES[name],lines=[start,end],source_commit='100adccc037d9f6898080e1cad58fcfc43364775')
    cases=[]
    def add(name,from_,to,bits,preset,radius,model,origins,edge=None,arrival=None,note=''):
        row=[*from_,*to,bits,preset,radius,model]
        h.fixed_selectors(' '.join(map(str,row)))
        cases.append(dict(id=name,row=row,origins=origins,requirement_edge=edge,
                          posthoc_source_arrival_tolerance=arrival,
                          status='PROPOSED NATIVE ONLY; not executed or predicted',note=note))
    tele=[origin('e2e/nav-tele-smoke.ts',11,14),origin('e2e/nav-tele-smoke.ts',49,54),origin('e2e/nav-tele-smoke.ts',86,122)]
    for name,bits,preset in [('enabled',1,4),('disabled',0,4),('missing-runes',1,1)]:
        add('spell-varrock-'+name,[3222,3218,0],[3213,3424,0],bits,preset,4,0,tele,2143,8,
            'Magic exactly 25 minimum adaptation, NOT source maxme. No packed quest/varp/worn req. Radius 4 is not arrival tolerance 8.')
    glory=[origin('e2e/nav-stress-live.ts',237,247),origin('e2e/nav-stress-live.ts',486,504)]
    for name,bits,preset in [('carried',1,5),('disabled',0,5),('worn-not-carried',1,2),('missing',1,0)]:
        add('glory-edgeville-'+name,[3293,3174,0],[3087,3496,0],bits,preset,4,0,glory,2166,12,
            'Stress source post-hoc glory tolerance is 12 (not tele-smoke 8); both excluded from router acceptance. Requested radius 4 retained.')
    add('mainland-stairs-f2p01',[3222,3218,0],[3208,3220,2],0,3,0,0,
        [origin('tools/nav/mainland-routes.json',4,4)],note='Packed stair/reachability intent, no live stair assertion.')
    add('mainland-f2p06',[3213,3424,0],[3222,3218,0],0,3,4,1,
        [origin('tools/nav/mainland-routes.json',9,9)],note='Root-selected radius4 and walking model adaptation of mainland OD.')
    for name,from_,to,edge,start,end in [('sarim-musa',[3027,3218,0],[2956,3143,1],2089,158,164),('ardy-brim',[2683,3272,0],[2775,3234,1],2095,165,171)]:
        for suffix,preset in [('coins5000',6),('coins10',2),('missing-coins',0)]:
            add('ship-'+name+'-'+suffix,from_,to,0,preset,0,1,
                [origin('tools/nav/transport-heavy-routes.ts',start,end),origin('tools/nav/transport-heavy-routes.ts',188,218)],edge,
                note='Coins-only adaptation, fare30. No packed skill/quest/varp/worn req. Source OD is near, not equal to packed NPC at. Ship selection remains an intent.')
    for suffix,bits,preset in [('fetch-missing',5,0),('fetch-worn-only',5,2),('bank-disabled',1,0),('already-carried',5,5)]:
        add('HOST-glory-karamja-'+suffix,[3222,3218,0],[2918,3176,0],bits,preset,0,0,tele[:1],2167,
            note='HOST-SPECIFIC proposal combines source Lumbridge origin with saved packed Glory Karamja destination. Fixed bank[(995,10),(1712,1)]. BankSession only after actual missing-item diagnosis/plan_bank_fetch output; BANK is not a graph edge.')
    assert len(cases)==len({c['id'] for c in cases})
    text=''.join(' '.join(map(str,c['row']))+'\n' for c in cases)
    h.fixed_selectors(text)
    dest.mkdir(parents=True,exist_ok=False)
    (dest/'routes.tsv').write_text(text)
    h.save(dest/'cases.json',dict(scope='Static proposals only; NO real input execution or native release',
        frozen=dict(dense=h.BASE,tiled=h.CANDIDATE,client=h.CLIENT),
        row_schema='from_x from_z from_level to_x to_z to_level option_bits state_family radius model',
        presets={'4':{'name':'varrock-smoke-min25','inv':[[563,50],[556,150],[554,50]],'stats':[[6,25]]},
                 '5':{'name':'carried-glory4','inv':[[1712,1]]},'6':{'name':'ship-coins5000','inv':[[995,5000]]}},
        omitted_preset_fields='All other inv/worn/stats/quests/varps empty; presets0..3 unchanged',
        bank=[[995,10],[1712,1]],
        exclusions=['membership','freeSlots','distanceBeforeTeleport','allowTeleportIds','foreign router policy',
                    'live logs/paint/arrival/pacing/account reproduction','entry-derived essence roundtrip'],
        essence_limit='Existing opts8 preseeded Aubury exit proof unchanged, not an entry-created full session roundtrip. No new essence rows.',
        requirements=anchors,case_count=len(cases),cases=cases,routes_sha256=h.sha(dest/'routes.tsv')))
    print(json.dumps(dict(path=str(dest),case_count=len(cases),routes_sha256=h.sha(dest/'routes.tsv'),cases_sha256=h.sha(dest/'cases.json'))))

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--source',type=Path,required=True);p.add_argument('--out',type=Path,required=True);a=p.parse_args()
    propose(a.source.resolve(),a.out.resolve())
