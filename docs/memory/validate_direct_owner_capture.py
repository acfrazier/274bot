"""Fail-closed offline direct-owner-v1 protocol validation. Never launches a process.

The sidecar is logical requested storage, NOT RSS. Native qualification also
requires the separate root admission (including guards/workload/cleanup proofs).
"""
import argparse
import hashlib
import json
from pathlib import Path

SCHEMA = 'direct-owner-v1'
CAPS = dict(owner_jsonl=262144, run_output=67108864, visits=262144,
            fragment_ns=5000000, rows=128, scratch=524288, mailbox=262144)
SNAPSHOT = set('struct_header npc players stats inv chat varps scene.collision_flags loc ground_item inventory equipment bank bank_side trade.my_offer trade.their_offer trade.side_pack trade.partner shop.stock widgets side_tabs chat_lines chat_options make_products quest_statuses menu_entries main_modal_texts chat_modal_texts login_message'.split())
FINGERPRINT = set('struct_header inv bank bank_side equipment trade_mine trade_theirs trade_side shop_stock npcs locs players ground booths varps side_tab_ifaces banks stats chat_lines spell_buttons combat_styles make_products nearest_booth chat_options chat_text my_name trade_partner'.split())
NUMBERS = 'element_len element_capacity occupied_count occupied_element_bytes capacity_bytes nested_capacity_bytes box_count box_bytes'.split()
CLIENT = set('struct_header groundh mapl collision.flags players npc local_player player_ids npc_ids entity_removal_ids entity_update_ids stat_base_level stat_effective_level stat_xp var var_serv player_appearance_buffer ground_obj stream_queues packet_pools audio loc_model_descendants ifaces_immutable_nested'.split())
OPAQUE = {('client', n) for n in 'ground_obj stream_queues packet_pools audio loc_model_descendants ifaces_immutable_nested'.split()} | {('ifaces', 'immutable_nested'), ('slotscript', 'ipc_builder_capacity'), ('slotscript', 'compiled_or_isolate_native'), ('fingerprint', 'builder_capacity')}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def uint(value, label):
    require(type(value) is int and 0 <= value <= 2**64 - 1, label + ': invalid unsigned integer')
    return value


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, 'duplicate JSON key: ' + key)
        result[key] = value
    return result


def validate_rows(rows):
    require(isinstance(rows, list) and len(rows) <= 32, 'record count')
    requests, fragments, encoded, stops, terminal = {}, {}, {}, [], []
    token = None
    for ordinal, row in enumerate(rows):
        require(isinstance(row, dict) and row.get('schema') == SCHEMA, 'schema')
        kind = row.get('kind')
        if kind in ('owner_request', 'owner_fragment', 'encoded_buf_meta'):
            require('slot_token' in row, 'missing slot identity')
        if 'slot_token' in row:
            current = uint(row['slot_token'], 'token')
            require(current > 0, 'zero token')
            token = current if token is None else token
            require(token == current, 'identity drift: slot token')
        if kind == 'owner_request':
            p = uint(row.get('phase'), 'phase')
            require(p < 3 and p not in requests, 'duplicate/invalid phase request')
            require(row.get('request_id') == p + 1, 'request/phase mismatch')
            issued = uint(row.get('issued_ns'), 'issued')
            require(row.get('deadline_ns') == issued + 1_000_000_000, 'deadline cap drift')
            requests[p] = row
        elif kind == 'owner_fragment':
            p, source = row.get('phase'), row.get('source')
            require(type(p) is int and p in requests, 'missing request')
            require(source in ('nav_snapshot', 'slotscript'), 'source')
            key = (p, source)
            require(key not in fragments, 'duplicate fragment')
            require(row.get('request_id') == p + 1, 'wrong request epoch')
            require(uint(row.get('frame_serial'), 'frame') > 0, 'zero frame')
            begin, end = uint(row.get('begin_ns'), 'begin'), uint(row.get('end_ns'), 'end')
            require(requests[p]['issued_ns'] <= begin <= end <= requests[p]['deadline_ns'], 'stale/deadline epoch')
            require(end - begin <= CAPS['fragment_ns'] and uint(row.get('visits'), 'visits') <= CAPS['visits'], 'fragment cap drift')
            require(row.get('complete') is True and row.get('reason') == 'ok', 'incomplete fragment')
            fields = row.get('rows')
            require(isinstance(fields, list) and 0 < len(fields) <= CAPS['rows'], 'field row cap')
            seen = set()
            for field in fields:
                require(isinstance(field, dict), 'malformed field')
                keyfield = (field.get('owner'), field.get('field'))
                require(all(isinstance(x, str) and x for x in keyfield) and keyfield not in seen, 'duplicate/malformed family')
                seen.add(keyfield)
                uint(field.get('elapsed_observer_ns'), 'field elapsed')
                if field.get('reason') == 'opaque_unknown':
                    require(keyfield in OPAQUE, 'unexpected unknown family')
                    require(field.get('complete') is False and all(field.get(n) is None for n in NUMBERS), 'unknown must be null')
                else:
                    require(keyfield not in OPAQUE, 'opaque family reported as measured')
                    require(field.get('complete') is True and field.get('reason') == 'ok', 'incomplete field')
                    for name in NUMBERS:
                        uint(field.get(name), name)
                    require(field['element_len'] <= field['element_capacity'], 'length exceeds capacity')
                    require(field['occupied_element_bytes'] <= field['capacity_bytes'], 'occupied exceeds capacity')
            if source == 'nav_snapshot':
                for owner in ('host_snapshot', 'nav_snapshot'):
                    require(SNAPSHOT <= {f for o, f in seen if o == owner}, 'missing ' + owner + ' family')
                for owner, required in {
                    'world': set('struct_header groundh squares sprites dynamic_sprites free_dynamic_sprites occluders occlusion_cycle'.split()),
                    'client': CLIENT,
                    'ifaces_mut': set('outer template_entries private_entries shared_entries'.split()),
                }.items():
                    require(required <= {f for o, f in seen if o == owner}, 'missing ' + owner + ' family')
                epochs = row.get('epochs')
                require(isinstance(epochs, list) and len(epochs) == 3, 'epoch count')
                for epoch, expected in zip(epochs, ('client', 'host_snapshot', 'nav_snapshot')):
                    require(isinstance(epoch, dict) and epoch.get('source') == expected, 'source epoch mismatch')
                    require(epoch.get('ingame') is True and epoch.get('scene_state') == 2, 'not ready epoch')
                    require(isinstance(epoch.get('gens'), list) and len(epoch['gens']) == 11, 'generation fields')
                    for g in epoch['gens']:
                        uint(g, 'generation')
                    require(isinstance(epoch.get('base'), list) and len(epoch['base']) == 2, 'missing base')
                    require(isinstance(epoch.get('tile'), list) and len(epoch['tile']) == 3, 'missing tile')
                    if expected != 'client':
                        require(isinstance(epoch.get('family_gates'), list) and len(epoch['family_gates']) == 25, 'missing family gates')
                require(epochs[0].get('draw') is False, 'draw enabled')
            else:
                require(row.get('script_state') == ('Idle' if p == 2 else 'Running'), 'wrong script state')
                require(row.get('fingerprint_present') is (p != 2), 'wrong fingerprint epoch')
                required = {'absent', 'builder_capacity'} if p == 2 else FINGERPRINT
                require(required <= {f for o, f in seen if o == 'fingerprint'}, 'missing fingerprint family')
                require({'pending_logs', 'last_error', 'ipc_builder_capacity', 'compiled_or_isolate_native'} <= {f for o, f in seen if o == 'slotscript'}, 'missing script family')
            fragments[key] = row
        elif kind == 'encoded_buf_meta':
            key = (row.get('buf_kind'), row.get('request_id'))
            require(key in ((0, 0), (1, 0), (2, 1), (2, 2)) and key not in encoded, 'duplicate/invalid encoded metadata')
            require(uint(row.get('len'), 'encoded length') <= uint(row.get('capacity'), 'encoded capacity'), 'encoded capacity')
            when = uint(row.get('mono_ns'), 'encoded time')
            uint(row.get('frame_serial'), 'encoded frame')
            if key[0] == 1:
                require((0, 0) in encoded, 'delta before initial')
            if key[0] == 2:
                require(key[1] - 1 in requests, 'unrequested buffer')
                start = requests[key[1] - 1]['issued_ns']
                require(start <= when <= start + 2_000_000_000, 'encoded window expired')
            encoded[key] = row
        elif kind == 'script_stop':
            require(not stops, 'duplicate Stop')
            require(uint(row.get('begin_ns'), 'Stop begin') <= uint(row.get('end_ns'), 'Stop end'), 'Stop order')
            stops.append(row)
        elif kind == 'owner_terminal':
            require(not terminal and ordinal == len(rows) - 1, 'terminal order/duplicate')
            require(row.get('complete') is True and row.get('reason') == 'ok', 'failed terminal')
            terminal.append(row)
        else:
            raise ValueError('unknown record kind')
    require(len(requests) == 3 and len(fragments) == 6 and len(encoded) == 4 and len(stops) == len(terminal) == 1, 'missing mandatory records')
    frames = []
    for p in range(3):
        a, b = fragments[p, 'nav_snapshot'], fragments[p, 'slotscript']
        require(a['frame_serial'] == b['frame_serial'] and a['end_ns'] <= b['begin_ns'], 'wrong frame or fragment order')
        frames.append(a['frame_serial'])
    require(frames[0] < frames[1] < frames[2], 'stale frame reuse')
    require(requests[0]['issued_ns'] < requests[1]['issued_ns'] < stops[0]['begin_ns'] <= stops[0]['end_ns'] < requests[2]['issued_ns'], 'phase/Stop order')
    return {'schema': SCHEMA, 'protocol_complete': True, 'requests': 3, 'fragments': 6, 'encoded': 4, 'rss_reconciliation': False, 'native_qualified': False}


def validate_file(path, expected_sha256):
    path = Path(path)
    require(not path.is_symlink() and path.is_file(), 'not a regular owner file')
    with path.open('rb') as stream:
        data = stream.read(CAPS['owner_jsonl'] + 1)
    require(len(data) <= CAPS['owner_jsonl'], 'owner output cap')
    require(hashlib.sha256(data).hexdigest() == expected_sha256, 'identity drift: owner bytes')
    require(data.endswith(b'\n'), 'truncated final line')
    rows = [json.loads(line, object_pairs_hook=unique_object) for line in data.splitlines()]
    return validate_rows(rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('jsonl', type=Path)
    parser.add_argument('--expected-sha256', required=True)
    args = parser.parse_args()
    try:
        result = validate_file(args.jsonl, args.expected_sha256)
    except (ValueError, TypeError, KeyError, OSError) as error:
        print(json.dumps({'protocol_complete': False, 'error': str(error)}))
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
