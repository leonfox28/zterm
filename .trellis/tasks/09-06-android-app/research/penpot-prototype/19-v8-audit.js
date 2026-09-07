// Read-only v8 audit; safe to repeat on the recorded document.
const D=storage.zd;
if(penpot.currentPage.id!=='c828d3cf-7d4e-8145-8008-98f4fa037d49'||!D.v8.polished)throw new Error('Wrong page/order');
const entries=Object.entries(D.boards),boards=entries.map(([,b])=>b),ids=new Set(boards.map(b=>b.id));
const all=boards.flatMap(b=>[b,...penpotUtils.findShapes(()=>true,b)]);
const interactions=all.flatMap(s=>s.interactions.map(i=>({shape:s.name,type:i.action.type,destination:i.action.destination?.id})));
const textOverflow=entries.flatMap(([key,b])=>penpotUtils.findShapes(s=>s.type==='text',b)
  .filter(s=>!penpotUtils.isContainedIn(s,b)).map(s=>({board:key,shape:s.name})));
const invalidDestinations=interactions.filter(i=>['navigate-to','open-overlay'].includes(i.type)&&!ids.has(i.destination));
return {
  pageId:penpot.currentPage.id,fileId:'c828d3cf-7d4e-8145-8008-98f4fa037d48',revision:8,
  boards:entries.map(([key,b])=>({key,id:b.id,name:b.name,width:b.width,height:b.height,preview:b.showInViewMode,...(key==='album'?{role:'editor-only system handoff'}:{})})),
  interactions:interactions.length,textOverflow,invalidDestinations,
  flows:penpot.currentPage.flows.map(f=>({name:f.name,start:f.startingBoard.id})),
  shortcutRows:entries.filter(([,b])=>b.children.some(s=>s.name==='Persistent shortcut surface')||b.id===D.boards.keyboard.id).map(([key,b])=>({key,rows:b.children.filter(s=>s.name==='Terminal shortcuts / fixed eight keys').map(s=>({keys:s.children.length,y:s.y-b.y,width:s.width,opacity:s.opacity})),gridBottom:Math.max(0,...b.children.filter(s=>s.name.startsWith('Terminal row / ')).map(s=>s.y-b.y+s.height))})),
  homeTargets:['home','homeNoRecent','homeRemoved'].map(key=>({key,cards:D.boards[key].children.filter(s=>s.name==='Recent connection'||s.name.startsWith('Host / ')).map(s=>({name:s.name,actions:s.interactions.map(i=>({type:i.action.type,to:i.action.destination?.id}))}))})),
  fontDialogs:entries.filter(([key])=>key.startsWith('font_')).map(([key,b])=>({key,selected:b.children.filter(s=>s.name.endsWith('/ selected')).length})),
  limitation:'Static click/state illustrations; no Android runtime, persisted preferences, OS permissions/picker, network, clipboard, or true touch gesture execution.'
};
