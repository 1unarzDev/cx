#!/usr/bin/env python3
"""Bounded preview responses through the real framed helper, using inert fixtures."""
import base64, io, json, os, pathlib, struct, subprocess, sys, tarfile, tempfile, zlib
binary = str(pathlib.Path(sys.argv[1]).resolve())
checks = []
with tempfile.TemporaryDirectory(prefix='cx-preview-helper-') as temporary:
    root = pathlib.Path(temporary)
    env = dict(os.environ, HOME=str(root), XDG_STATE_HOME=str(root/'state'))
    def preview(name, body):
        path = root/name; path.write_bytes(body)
        request = json.dumps(dict(version=1, id='preview-fixture', op=dict(op='preview', args=dict(path=str(path))))).encode()
        result = subprocess.run([binary, 'helper'], input=b'CX1 '+str(len(request)).encode()+b'\n'+request, env=env, capture_output=True, timeout=8)
        assert result.returncode == 0, result.stderr
        header, data = result.stdout.split(b'\n', 1)
        assert header == b'CX1 '+str(len(data)).encode() and len(data) <= 1024*1024
        response = json.loads(data); assert response['error'] is None, response['error']
        return response['result']
    text = preview('text.txt', b'one\r\ntwo\rthree\tend\x1b[31m')
    assert 'one\ntwo\nthree' in text['text'] and '\r' not in text['text'] and '\x1b' not in text['text']
    assert preview('doc.md', b'# Heading\n**bold**')['kind'] == 'markdown'
    checks.append('CRLF/CR normalized and terminal controls escaped; Markdown classified')
    def chunk(kind, data):
        return struct.pack('!I', len(data))+kind+data+struct.pack('!I', zlib.crc32(kind+data))
    png = b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR', struct.pack('!2I5B', 1, 1, 8, 6, 0, 0, 0))+chunk(b'IDAT', zlib.compress(b'\0\xff\0\0\xff'))+chunk(b'IEND', b'')
    image = preview('pixel.png', png)['image']
    assert base64.b64decode(image['rgba']) == b'\xff\0\0\xff'
    assert preview('broken.png', png[:18])['image'] is None
    width, height = 320, 200
    rows = b''.join(b'\0'+bytes([row % 256, 40, 180, 255])*width for row in range(height))
    large_png = b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR', struct.pack('!2I5B', width, height, 8, 6, 0, 0, 0))+chunk(b'IDAT', zlib.compress(rows))+chunk(b'IEND', b'')
    large = preview('large.png', large_png)['image']
    assert (large['width'], large['height']) == (width, height)
    assert base64.b64decode(large['png']).startswith(b'\x89PNG\r\n\x1a\n')
    assert len(large['png']) <= 800000
    checks.append('real bundled PNG decode, preserved high resolution, and corrupt image fallback')
    pdf = bytearray(b'%PDF-1.4\n'); offsets = []
    bodies = ['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R] /Count 1 >>', '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Resources << >> /Contents 4 0 R >>', '<< /Length 0 >>\nstream\n\nendstream']
    for index, body in enumerate(bodies, 1):
        offsets.append(len(pdf)); pdf.extend(f'{index} 0 obj\n{body}\nendobj\n'.encode())
    xref = len(pdf); pdf.extend(b'xref\n0 5\n0000000000 65535 f \n')
    for offset in offsets: pdf.extend(f'{offset:010} 00000 n \n'.encode())
    pdf.extend(f'trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode())
    page = preview('page.pdf', pdf)
    assert page['kind'] == 'pdf' and page['image'] is not None and page['title'] == 'PDF · page 1' and max(page['image']['width'], page['image']['height']) > 160
    checks.append('installed Poppler renders real PDF page through helper')
    archive = io.BytesIO()
    with tarfile.open(fileobj=archive, mode='w', format=tarfile.PAX_FORMAT) as tar:
        entry = tarfile.TarInfo('a'*1_100_000); entry.size = 0; tar.addfile(entry)
    listing = preview('huge-name.tar', archive.getvalue())
    assert listing['kind'] == 'archive' and listing['truncated'] and len(listing['text']) < 32768
    checks.append('hostile PAX filename cannot overflow framed response')
print(json.dumps(dict(result='PASS', checks=checks)))
