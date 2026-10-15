import http.server, json, threading, pathlib, time
root=pathlib.Path('/tmp/controlla-audit-20261014')
root.mkdir(parents=True, exist_ok=True)
events=[]
class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self,*a): pass
    def do_GET(self):
        if self.path.startswith('/state'):
            data=json.dumps(events).encode(); typ='application/json'
        elif self.path.startswith('/download'):
            data=b'Controlla audit download sentinel\n'; typ='text/plain'
        elif self.path.startswith('/frame'):
            data=b'<title>Audit child frame</title><button id="child" onclick="this.textContent=\'Child clicked\'">Child button</button>';typ='text/html'
        else:
            data=HTML.replace('FRAME_PORT',str(frame.server_port)).encode();typ='text/html'
        self.send_response(200);self.send_header('Content-Type',typ)
        if self.path.startswith('/download'):self.send_header('Content-Disposition','attachment; filename="controlla-audit.txt"')
        self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
    def do_POST(self):
        raw=self.rfile.read(min(int(self.headers.get('Content-Length','0')),1048576))
        try: value=json.loads(raw)
        except: value={'bytes':len(raw)}
        events.append({'time_ns':time.monotonic_ns(),'path':self.path,'value':value})
        (root/'events.json').write_text(json.dumps(events,indent=2))
        self.send_response(200);self.end_headers();self.wfile.write(b'accepted')
HTML='''<!doctype html><meta charset="utf-8"><title>Controlla capability audit</title>
<style>body{font:18px Arial;padding:20px}button,input,textarea,select{margin:10px;padding:10px}#editor,#drop,#drag{border:2px solid #444;padding:15px;width:240px;margin:8px}iframe{margin-left:100px;width:400px;height:120px}</style>
<h1>Controlla capability audit</h1><div id="account">Audit test account</div>
<button id="click">Click counter</button><output id="count">Count 0</output><button id="touch">Touch counter</button><output id="touchcount">Touch 0</output><br>
<label>Plain text<input id="plain" aria-label="Plain text"></label><label>Human text<input id="human" aria-label="Human text"></label><br>
<div id="editor" role="textbox" aria-label="Rich editor" contenteditable="true"></div>
<label>Choice<select id="choice" aria-label="Choice"><option>A</option><option>B</option></select></label>
<input id="slider" aria-label="Volume" type="range" min="0" max="100" value="20"><output id="volume">20</output><br>
<div id="drag" draggable="true">Drag item</div><div id="drop" role="region" aria-label="Drop zone">Drop zone</div>
<input id="file" type="file" aria-label="Upload audit file"><output id="fileout">No file</output>
<a href="/download" download>Download audit file</a><br>
<iframe title="Cross origin test frame" src="http://localhost:FRAME_PORT/frame"></iframe>
<div style="height:1000px">Scroll space</div><button id="bottom">Bottom button</button><output id="bottomout">Not clicked</output>
<script>
const log=(kind,value,e)=>fetch('/event',{method:'POST',body:JSON.stringify({kind,value,trusted:e?.isTrusted,visible:document.visibilityState,focused:document.hasFocus()})});
let n=0,t=0;click.onclick=e=>{count.textContent='Count '+(++n);log('click',n,e)};touch.onclick=e=>{touchcount.textContent='Touch '+(++t);log('touch',t,e)};
for(const id of ['plain','human','editor','choice','slider'])document.getElementById(id).addEventListener('input',e=>{volume.textContent=slider.value;log(id,e.target.value??e.target.textContent,e)});
drag.ondragstart=e=>e.dataTransfer.setData('text/plain','audit-item');drop.ondragover=e=>e.preventDefault();drop.ondrop=e=>{e.preventDefault();drop.textContent=e.dataTransfer.getData('text/plain');log('drop',drop.textContent,e)};
file.onchange=async e=>{let f=file.files[0];if(f){await fetch('/upload',{method:'POST',body:await f.text()});fileout.textContent='Accepted '+f.name;log('upload',f.name,e)}};
bottom.onclick=e=>{bottomout.textContent='Bottom clicked';log('bottom',true,e)};
</script>'''
frame=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
main=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
threading.Thread(target=frame.serve_forever,daemon=True).start()
(root/'ports.json').write_text(json.dumps({'main':main.server_port,'frame':frame.server_port}))
print(json.dumps({'main':main.server_port,'frame':frame.server_port}),flush=True)
main.serve_forever()
