#!/usr/bin/env python3
"""Three distinct PDF pages through the real descriptor-safe framed helper."""
import base64, hashlib, json, os, pathlib, subprocess, sys, tempfile
binary = str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-pdf-pages-') as directory:
    root = pathlib.Path(directory)
    objects = ['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R 5 0 R 7 0 R] /Count 3 >>']
    for index, color in enumerate(['1 0 0', '0 1 0', '0 0 1']):
        objects.append(f'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Resources << >> /Contents {4+index*2} 0 R >>')
        content = f'{color} rg 0 0 200 100 re f\n'
        objects.append(f'<< /Length {len(content)} >>\nstream\n{content}endstream')
    data = bytearray(b'%PDF-1.4\n'); offsets = []
    for index, body in enumerate(objects, 1):
        offsets.append(len(data)); data.extend(f'{index} 0 obj\n{body}\nendobj\n'.encode())
    xref = len(data); data.extend(f'xref\n0 {len(objects)+1}\n0000000000 65535 f \n'.encode())
    for offset in offsets: data.extend(f'{offset:010} 00000 n \n'.encode())
    data.extend(f'trailer\n<< /Size {len(objects)+1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode())
    path = root/'- Unicode résumé "quoted" \x1b[31m.pdf'; path.write_bytes(data)
    original = hashlib.sha256(data).hexdigest()
    def request(op='preview_page', page=1, file=path, overrides=None):
        args = dict(path=str(file))
        if op == 'preview_page': args['page'] = page
        payload = json.dumps(dict(version=1, id=f'page-{page}', op=dict(op=op,args=args))).encode()
        env = dict(os.environ, HOME=str(root), XDG_STATE_HOME=str(root/'state'))
        if overrides: env.update(overrides)
        result = subprocess.run([binary,'helper'], input=b'CX1 '+str(len(payload)).encode()+b'\n'+payload, capture_output=True, env=env, timeout=8)
        assert result.returncode == 0, result.stderr
        header, body = result.stdout.split(b'\n',1)
        assert header == b'CX1 '+str(len(body)).encode() and len(body) <= 1024*1024
        response=json.loads(body); assert response['id']==f'page-{page}'
        return response
    hashes=[]
    for page in range(1,4):
        response=request(page=page); assert response['error'] is None,response['error']
        view=response['result']; assert view['page']==page and view['pages']==3
        assert view['title']==f'PDF · page {page}' and '\x1b' not in view['title']
        image=base64.b64decode(view['image']['png'],validate=True)
        assert image.startswith(b'\x89PNG\r\n\x1a\n')
        hashes.append(hashlib.sha256(image).hexdigest())
    assert len(set(hashes))==3
    for page in (0,4,10001): assert request(page=page)['error']
    legacy=request(op='preview')['result']; assert legacy['page']==1 and legacy['pages']==3
    assert hashlib.sha256(path.read_bytes()).hexdigest()==original
    text=root/'not-a-pdf';text.write_text('plain text')
    assert request(file=text)['error']
    corrupt=root/'corrupt.pdf';corrupt.write_bytes(b'%PDF-1.4 broken')
    assert request(file=corrupt)['error']
    assert request(file=path,overrides={'PATH':str(root/'no-converter')})['error']
    unavailable=request(op='preview',file=path,overrides={'PATH':str(root/'no-converter')})['result']
    assert unavailable['image'] is None and unavailable['pages'] is None and 'not available' in unavailable['text']
print(json.dumps(dict(result='PASS',pages=3,distinct_page_pngs=3,source_integrity='unchanged',bounds=[0,4,10001],checks=['real framed helper','Unicode/control/quoted filename','legacy preview page 1','non-PDF/malformed rejection','missing converter honest fallback'])))
