# SPDX-License-Identifier: Apache-2.0
"""Export Zephyr's trusted, build-generated edt.pickle for the Rust generator.

The versioned interchange format preserves raw properties, including properties
without bindings. Only this adapter depends on edtlib/dtlib's Python internals.
"""
import argparse
import json
import pickle
import sys
from pathlib import Path


def property_values(prop):
    values = []
    words = None
    width = 4
    # dtlib markers preserve mixed values and the distinction between numbers
    # and references, which cannot be recovered from the flattened bytes alone.
    markers = [m for m in prop._markers if m[1].name != "LABEL"]
    for i, (pos, kind, _) in enumerate(markers):
        end = markers[i + 1][0] if i + 1 < len(markers) else len(prop.value)
        kind = kind.name
        if kind == "STRING":
            values.append({"String": prop.value[pos:end - 1].decode("utf-8")})
        elif kind == "PATH":
            values.append({"Phandle": prop.to_path().path})
        else:
            if kind.startswith("UINT"):
                width = int(kind[4:]) // 8
                if width == 1:
                    values.append({"Bytes": list(prop.value[pos:end])})
                    continue
                words = []
                values.append({"Words": words})
            elif kind == "PHANDLE":
                target = prop.node.dt.phandle2node[int.from_bytes(prop.value[pos:pos + 4], "big")]
                words.append({"Phandle": target.path})
                pos += 4
            for offset in range(pos, end, width):
                number = int.from_bytes(prop.value[offset:offset + width], "big")
                # The runtime word representation is u32. Preserve wide values
                # as big-endian cells rather than truncating them.
                if width == 8:
                    words.append({"Number": number >> 32})
                words.append({"Number": number & 0xffffffff})
    return values


def export(edt):
    def node(entry):
        return dict(name=entry.name, path=entry.path, ord=entry.dep_ordinal,
                    labels=entry.labels,
                    properties=[dict(name=p.name, value=property_values(p))
                                for p in entry._node.props.values()],
                    children=[node(child) for child in entry.children.values()])
    return {"version": 1, "root": node(edt.get_node("/"))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--zephyr-base", required=True, type=Path)
    parser.add_argument("--edt-pickle", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    sys.path.insert(0, str(args.zephyr_base / "scripts/dts/python-devicetree/src"))
    with args.edt_pickle.open("rb") as source:
        result = export(pickle.load(source))
    text = json.dumps(result, ensure_ascii=True, indent=2) + "\n"
    if not args.output.exists() or args.output.read_text() != text:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(text)


if __name__ == "__main__":
    main()
