These JAG archives are synthetic test data authored by `generate.py`; they contain
no downloaded game content. Regenerate with `python3 generate.py`.

The config archive contains one named object, NPC and location. The interface
archive contains one 1x1 layer. The version list has empty model/animation/music/
map tables. Other archives are valid empty JAG containers. The two manifests
intentionally declare the same synthetic bytes for different revisions so tests
can exercise real shared-cache decoding and profile binding without a local
engine. They do not qualify a real cache, navigation pack, or live game session.
