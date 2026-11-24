"""Converts an OpenCV LBP cascade (new XML format) into the compact binary
file RoomGuard embeds. Run: python tools/convert_cascade.py in.xml out.bin

Layout (little endian):
  b"RGLBP1\0\0"
  u32 window_w, u32 window_h
  u32 n_features, then per feature: i32 x, y, w, h  (one cell of the 3x3 grid)
  u32 n_stages, then per stage: f32 threshold, u32 n_weak,
      then per weak classifier: u32 feature, i32 subset[8], f32 leaf_hit, f32 leaf_miss
"""
import struct, sys
import xml.etree.ElementTree as ET

def load(path):
    root = ET.parse(path).getroot()
    c = root.find('cascade')
    w = int(c.find('width').text); h = int(c.find('height').text)
    stages = []
    for st in c.find('stages').findall('_'):
        thr = float(st.find('stageThreshold').text)
        weaks = []
        for wk in st.find('weakClassifiers').findall('_'):
            nodes = [int(v) for v in wk.find('internalNodes').text.split()]
            leaves = [float(v) for v in wk.find('leafValues').text.split()]
            assert nodes[0] == 0 and nodes[1] == -1 and len(nodes) == 11, nodes
            weaks.append((nodes[2], nodes[3:11], leaves[0], leaves[1]))
        stages.append((thr, weaks))
    feats = []
    for f in c.find('features').findall('_'):
        x, y, fw, fh = [int(v) for v in f.find('rect').text.split()]
        feats.append((x, y, fw, fh))
    return w, h, feats, stages

def write(path, w, h, feats, stages):
    out = bytearray(b"RGLBP1\0\0")
    out += struct.pack('<II', w, h)
    out += struct.pack('<I', len(feats))
    for f in feats:
        out += struct.pack('<iiii', *f)
    out += struct.pack('<I', len(stages))
    for thr, weaks in stages:
        out += struct.pack('<fI', thr, len(weaks))
        for fi, subset, l0, l1 in weaks:
            out += struct.pack('<I', fi)
            out += struct.pack('<8i', *subset)
            out += struct.pack('<ff', l0, l1)
    open(path, 'wb').write(bytes(out))

if __name__ == '__main__':
    w, h, feats, stages = load(sys.argv[1])
    write(sys.argv[2], w, h, feats, stages)
    print(f"window {w}x{h}, {len(feats)} features, {len(stages)} stages, "
          f"{sum(len(s[1]) for s in stages)} weak classifiers")
