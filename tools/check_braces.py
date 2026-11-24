"""Rough syntax sanity check for Rust sources without a compiler: balanced
brackets outside strings, chars and comments, plus a few common slips."""
import re, sys, pathlib

def scan(path):
    s = pathlib.Path(path).read_text()
    stack, i, line, n = [], 0, 1, len(s)
    pairs = {')': '(', ']': '[', '}': '{'}
    problems = []
    while i < n:
        c = s[i]
        if c == '\n':
            line += 1
        if s.startswith('//', i):
            j = s.find('\n', i)
            i = n if j < 0 else j
            continue
        if s.startswith('/*', i):
            j = s.find('*/', i)
            line += s[i:j].count('\n')
            i = j + 2
            continue
        if c == 'r' and re.match(r'r#*"', s[i:i+4]) and not (i > 0 and (s[i-1].isalnum() or s[i-1] == '_')):
            m = re.match(r'r(#*)"', s[i:])
            hashes = m.group(1)
            end = s.find('"' + hashes, i + len(m.group(0)))
            line += s[i:end].count('\n')
            i = end + 1 + len(hashes)
            continue
        if c == 'b' and i + 1 < n and s[i+1] == '"' and not (i > 0 and (s[i-1].isalnum() or s[i-1] == '_')):
            i += 1
            c = '"'
        if c == '"':
            j = i + 1
            while j < n and s[j] != '"':
                if s[j] == '\\':
                    j += 1
                if s[j] == '\n':
                    line += 1
                j += 1
            i = j + 1
            continue
        if c == "'":
            m = re.match(r"'(\\.|\\u\{[0-9a-fA-F]+\}|\\x[0-9a-fA-F]{2}|[^\\'])'", s[i:])
            if m:
                i += len(m.group(0))
                continue
            i += 1  # lifetime
            continue
        if c in '([{':
            stack.append((c, line))
        elif c in ')]}':
            if not stack or stack[-1][0] != pairs[c]:
                problems.append(f"{path}:{line}: unexpected {c} (open {stack[-1] if stack else None})")
                return problems
            stack.pop()
        i += 1
    if stack:
        problems.append(f"{path}: unclosed {stack[-3:]}")
    txt = s
    for pat, msg in [(r'as (u8|u16|u32|u64|usize|i32|i64) <<', 'as T << needs parentheses'),
                     (r'as (u8|u16|u32|u64|usize|i32|i64) <(?!=)', 'as T < needs parentheses'),
                     ('\u2014', 'em dash'), ('\u2013', 'en dash')]:
        for m in re.finditer(pat, txt):
            problems.append(f"{path}:{txt[:m.start()].count(chr(10))+1}: {msg}")
    return problems

bad = []
for p in sys.argv[1:]:
    bad += scan(p)
print("\n".join(bad) if bad else f"ok: {len(sys.argv)-1} files")
