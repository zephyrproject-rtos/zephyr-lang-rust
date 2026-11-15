# SPDX-License-Identifier: Apache-2.0
import importlib.util
import unittest
from types import SimpleNamespace as NS
from pathlib import Path

spec = importlib.util.spec_from_file_location("edt_export", Path(__file__).with_name("edt-export.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ExportTests(unittest.TestCase):
    def test_mixed_values_and_unlabeled_reference(self):
        prop = NS(
            value=b'hello\0' + bytes.fromhex('000000070000000300ff'),
            _markers=[(0, NS(name='STRING'), None),
                      (6, NS(name='UINT32'), None),
                      (6, NS(name='PHANDLE'), None),
                      (14, NS(name='UINT8'), None)],
            node=NS(dt=NS(phandle2node={7: NS(path='/gpio@0')})),
        )
        self.assertEqual(module.property_values(prop), [
            {'String': 'hello'},
            {'Words': [{'Phandle': '/gpio@0'}, {'Number': 3}]},
            {'Bytes': [0, 255]},
        ])

    def test_empty_and_wide_values(self):
        self.assertEqual(module.property_values(NS(value=b'', _markers=[])), [])
        prop = NS(value=bytes.fromhex('123456789abcdef0'),
                  _markers=[(0, NS(name='UINT64'), None)])
        self.assertEqual(module.property_values(prop), [
            {'Words': [{'Number': 0x12345678}, {'Number': 0x9abcdef0}]}])


if __name__ == '__main__':
    unittest.main()
