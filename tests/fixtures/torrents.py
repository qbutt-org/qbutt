"""Use libtorrent itself for deterministic v1/v2/hybrid torrent metadata."""

import json
import pathlib
import sys

import libtorrent as lt

if lt.__version__ != "2.0.14.0":
    raise RuntimeError("Install tests/fixtures/requirements.txt with --require-hashes")

root = pathlib.Path(sys.argv[1])
payload = json.loads((root / "payload.json").read_text(encoding="utf-8"))
descriptions = []
for name, piece_size, flags in [
    ("v1", 16384, lt.create_torrent.v1_only),
    ("v1-64k", 65536, lt.create_torrent.v1_only),
    ("v2", 16384, lt.create_torrent.v2_only),
    ("hybrid", 16384, 0),
]:
    files = lt.file_storage()
    for item in payload:
        files.add_file(item["path"], item["size"])
    creator = lt.create_torrent(files, piece_size, flags)
    creator.set_creator("qbutt integration fixtures")
    creator.set_comment("Generated test data; no third-party payload")
    creator.set_priv(True)
    lt.set_piece_hashes(creator, str(root / "seed"))
    metadata = creator.generate()
    # The creation timestamp is outside the info dictionary and has no hash semantics.
    metadata.pop(b"creation date", None)
    encoded = lt.bencode(metadata)
    torrent_path = root / (name + ".torrent")
    torrent_path.write_bytes(encoded)
    info = lt.torrent_info(encoded)
    storage = info.files()
    descriptions.append({
        "name": name,
        "file": torrent_path.name,
        "pieceLength": info.piece_length(),
        "pieceCount": info.num_pieces(),
        "infoHashV1": str(info.info_hashes().v1) if info.info_hashes().has_v1() else None,
        "infoHashV2": str(info.info_hashes().v2) if info.info_hashes().has_v2() else None,
        "files": [{
            "index": i,
            "path": storage.file_path(i).replace("\\", "/"),
            "size": storage.file_size(i),
            "offset": storage.file_offset(i),
            "pad": bool(storage.file_flags(i) & lt.file_storage.flag_pad_file),
        } for i in range(storage.num_files())],
    })
print(json.dumps({"libtorrent": lt.__version__, "torrents": descriptions}))
