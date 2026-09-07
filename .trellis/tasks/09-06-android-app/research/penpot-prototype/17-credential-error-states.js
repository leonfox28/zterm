// v8e: pending and error states of the same minimal credential dialog.
const D=storage.zd,C=D.C,V=D.v8;
if(penpot.currentPage.id!=='c828d3cf-7d4e-8145-8008-98f4fa037d49'||!D.boards.recoverFailed)throw new Error('Wrong page/order');
if(D.boards.ticketPending)throw new Error('v8e already applied');
const pending=V.clone('ticketPending','ticketReady','连接凭据 / 连接中',0,23200);
for(const s of pending.children){
  if(s.name==='Button / 连接'){V.unlink(s);s.name='Button / 连接中';s.children.find(t=>t.type==='text').characters='连接中…';s.fills=[{fillColor:C.panel2}];s.children.find(t=>t.type==='text').fills=[{fillColor:C.muted}];}
  if(s.name==='Touch / Dismiss credentials'){V.unlink(s);s.addInteraction('click',{type:'close-overlay',destination:pending});}
  if(s.name==='Touch / Simulate credential input')V.unlink(s);
}
pending.addInteraction('after-delay',{type:'navigate-to',destination:D.boards.paired},1200);
const connect=D.boards.ticketReady.children.find(s=>s.name==='Button / 连接');V.unlink(connect);
connect.addInteraction('click',{type:'close-overlay',destination:D.boards.ticketReady});V.open(connect,pending);
for(const [i,[key,label,flowName]] of [['ticketExpired','凭据已过期','扫码 · 凭据过期'],['ticketInvalid','凭据格式不正确','扫码 · 凭据无效']].entries()){
  const b=V.clone(key,'ticketReady','连接凭据 / '+label,420+i*420,23200);b.resize(342,304);
  for(const s of b.children){
    if(s.name==='Button / 连接'){V.unlink(s);s.y=b.y+232;s.fills=[{fillColor:C.panel2}];s.children.find(t=>t.type==='text').fills=[{fillColor:C.muted}];}
    if(s.name==='Touch / Dismiss credentials'){V.unlink(s);s.addInteraction('click',{type:'close-overlay',destination:b});}
    if(s.name==='Touch / Simulate credential input')s.remove();
  }
  D.text(b,24,187,label,13,C.text,294,28,400);
  const edit=V.hit(b,24,60,294,116,'Correct credential input');
  edit.addInteraction('click',{type:'close-overlay',destination:b});V.open(edit,D.boards.ticketReady);
  const base=V.clone(key+'Demo','scanner','扫码 / '+label+'示例',1260+i*420,23200);
  base.addInteraction('after-delay',{type:'open-overlay',destination:b,position:'center',closeWhenClickOutside:true,addBackgroundOverlay:true},400);
  penpot.currentPage.createFlow(flowName,base);
}
for(const f of penpot.currentPage.flows.filter(f=>/^Flow \d+$/.test(f.name)))f.remove();
return {pending:pending.id,expired:D.boards.ticketExpired.id,invalid:D.boards.ticketInvalid.id};
