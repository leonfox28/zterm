// Browser-review fixes: pending-button text width and neutral first-entry list.
const D=storage.zd,C=D.C;
if(penpot.currentPage.id!=='c828d3cf-7d4e-8145-8008-98f4fa037d49'||!D.v8.polished)throw new Error('Wrong page/order');
if(D.v8.previewFixed)throw new Error('v8 preview fixes already applied');
const button=D.boards.ticketPending.children.find(s=>s.name==='Button / 连接中');
button.children.find(s=>s.type==='text').remove();
D.text(button,8,10,'连接中…',15,C.muted,278,28,500,false,'center');
for(const s of D.boards.firstSessions.children)if(s.type==='text'&&s.characters==='android-app'&&s.y-D.boards.firstSessions.y>=86){s.fills=[{fillColor:C.text}];s.fontWeight='400';}
for(const key of ['firstEntry','firstEmpty','firstEmptyList','firstSessions'])for(const s of D.boards[key].children)if(['Icon / up','Icon / down'].includes(s.name))s.x=D.boards[key].x+89;
for(const s of D.boards.firstCreatedList.children)if(s.name==='Icon / up')s.x=D.boards.firstCreatedList.x+107;
D.v8.previewFixed=true;
return {fixed:true};
