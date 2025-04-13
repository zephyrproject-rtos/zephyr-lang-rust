#!/usr/bin/env python3

# SPDX-License-Identifier: Apache-2.0

import os
import sys
import argparse
import json

from itertools import chain

sys.path.insert(0, str(os.path.join(os.environ["ZEPHYR_BASE"], "scripts", "kconfig")))
from kconfiglib import Kconfig, _T_BOOL, _T_CHOICE


def main():
    args = parse_args()

    kconf = Kconfig(args.kconfig_file)
    kconf.load_config(args.dotconfig)

    options = {}

    for sym in chain(kconf.unique_defined_syms, kconf.unique_choices):
        if not sym.name:
            continue

        if "-" in sym.name:
            # Rust does not allow hyphens in cfg options.
            continue

        if sym.type in [_T_BOOL, _T_CHOICE]:
            options[f"CONFIG_{sym.name}"] = sym.str_value

    with open(args.outfile, "w") as f:
        json.dump(options, f, indent=2, sort_keys=True)


def parse_args():
    parser = argparse.ArgumentParser(allow_abbrev=False)

    parser.add_argument("kconfig_file", help="Top-level Kconfig file")
    parser.add_argument("dotconfig", help="Path to dotconfig file")
    parser.add_argument("outfile", help="Path to output file")

    return parser.parse_args()


if __name__ == "__main__":
    main()
