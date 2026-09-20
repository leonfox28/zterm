const U=storage.su;if(penpot.currentPage.id!=='bd580feb-afb2-80e5-8008-aabb538508fc')throw new Error('Wrong page');
const settings=Object.entries(U.boards).filter(([key])=>!key.startsWith('connection'));
for(const [key,b] of settings){const row=U.refs[key].update;if(!row)continue;for(const s of [...row.children])if(s.name==='Icon / update'||s.name==='Icon / right')s.remove();U.icon(row,'download',16,24,24,U.refs[key].C.text);}
for(const s of [...U.boards.latest.children])if(s.name.startsWith('Toast /'))s.remove();
const toast=U.box(U.boards.latest,88,768,214,48,'#F2F2F2',24,'System Toast / OS-owned illustration');
U.dot(toast,14,12,24,'#101713');U.icon(toast,'terminal',18,16,16,'#D9FBA7');U.text(toast,50,12,'已是最新版',14,'#202124',145,24);
const caption=penpotUtils.findShape(s=>s.type==='text'&&s.characters==='03 / 无更新：短暂 Toast',penpot.root);if(caption){caption.characters='03 / 无更新：Android 系统 Toast';caption.name=caption.characters;}
const n=U.box(null,940,2680,860,226,'#FFFFFF',20,'Review v2 / System Toast and actions');
U.text(n,28,20,'系统反馈与动作入口',23,'#203529',804,40,600);
U.text(n,28,77,'• “已是最新版”调用 Android 系统文字 Toast；原型仅示意。\n• Toast 的图标、配色、位置和时长由手机系统决定。\n• 检查更新换成统一的下载图标，去掉右侧导航箭头。\n• 检查中仍显示进度指示，不暗示会进入另一个设置页面。',15,'#53634F',804,126);
return {updatedRows:settings.length,toastId:toast.id};
