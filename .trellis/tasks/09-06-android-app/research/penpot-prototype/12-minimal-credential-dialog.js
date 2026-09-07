// v7 planning revision: only field, wide centered Connect, and top-right close.
const D=storage.zd;
if(penpot.currentPage.id!=='c828d3cf-7d4e-8145-8008-98f4fa037d49')throw new Error('Wrong page');
for(const key of ['ticket','ticketReady']){
  const b=D.boards[key];
  if(!b.children.some(s=>s.name==='Credential field')||!b.children.some(s=>s.name==='Button / 连接'))throw new Error('Unexpected credential dialog');
}
if(D.boards.ticket.height===272)throw new Error('v7 already applied');
for(const key of ['ticket','ticketReady']){
  const b=D.boards[key];
  for(const s of [...b.children]){
    if((s.type==='text'&&['连接凭据','粘贴'].includes(s.characters))||['Touch / Simulate paste credentials','Touch / Paste credentials again'].includes(s.name)){s.remove();continue;}
    if(s.name==='Credential field'||s.name==='Touch / Simulate credential input'){s.y=b.y+60;s.resize(294,116);}
    if(s.type==='text'&&(s.characters==='输入或粘贴凭据'||s.characters.startsWith('zterm-pair://')))s.y=b.y+74;
    if(s.name==='Icon / close')s.y=b.y+20;
    if(s.name==='Touch / Dismiss credentials')s.y=b.y+8;
    if(s.name==='Button / 连接'){s.x=b.x+24;s.y=b.y+200;s.resize(294,48);}
  }
  b.resize(342,272);
}
const guide=penpotUtils.findShape(s=>s.name==='Prototype guide',penpot.root);
const note=penpotUtils.findShape(s=>s.type==='text'&&s.characters.startsWith('①'),guide);
note.characters=note.characters.replace('输入 / 粘贴模拟填入后连接','点文本框模拟填入后连接');
penpot.selection=[D.boards.ticket];penpot.viewport.zoomIntoView([D.boards.scanner,D.boards.ticket]);
return {revision:7,dialogs:['ticket','ticketReady'].map(k=>({key:k,id:D.boards[k].id,width:D.boards[k].width,height:D.boards[k].height}))};
