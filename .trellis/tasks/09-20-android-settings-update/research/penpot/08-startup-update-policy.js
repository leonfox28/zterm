if (penpot.currentPage.id !== 'bd580feb-afb2-80e5-8008-aabb538508fc') throw new Error('Wrong design page');
const U = storage.su;
if (!U?.box || !U?.text) throw new Error('Design helpers unavailable; do not replay foundation creation');
const name = 'Review v3 / Startup update policy';
const existing = penpot.currentPage.root.children.find(s => s.name === name);
if (existing) return { alreadyExists: true, id: existing.id };
const note = U.box(null, 1880, 180, 860, 490, '#FFFFFF', 20, name);
note.setPluginData('ztermSettingsUpdate', 'startup-policy-approved-2026-09-20');
U.text(note, 28, 20, '启动检查 · 已确认的交互规则', 23, '#203529', 804, 40, 600);
U.text(note, 28, 67, '冷启动 → 首屏可用 → 静默检查 → 有新版时复用现有更新弹窗', 15, '#476A20', 804, 34, 500);
U.text(note, 28, 122, [
  '• 每次冷启动检查一次；切回前台、旋转屏幕不重复检查。',
  '• 无新版或检查失败：不显示加载层、Toast、通知或错误弹窗。',
  '• 有新版：前台且没有权限／错误弹窗时提示，确认后才下载。',
  '• 点击“稍后”：同一版本 24 小时内不再自动提醒，重启后仍有效。',
  '• 出现更高版本可再次提醒；手动检查始终绕过暂缓提醒。',
  '• 手动无新版仍显示系统 Toast；手动检查失败明确提示。',
  '• 更新弹窗与“终端通知”开关无关，不需要通知权限。',
  '• 不增加定时后台检查、自动下载或强制更新。'
].join('\n'), 15, '#53634F', 804, 260);
U.text(note, 28, 418, '复用画板 04／06 的弹窗样式。这里是交互说明；原型不模拟冷启动、24 小时持久计时或真实下载。', 13, '#53634F', 804, 48);
const toastNote = penpot.currentPage.root.children.find(s => s.name === 'Review v2 / System Toast and actions');
if (toastNote) {
  const body = toastNote.children.find(s => s.type === 'text' && s.characters.includes('已是最新版'));
  if (body) {
    body.characters = body.characters.replace('• “已是最新版”调用', '• 手动检查的“已是最新版”调用');
    body.name = 'Manual check / Native system Toast notes';
  }
}
const caption = penpot.currentPage.root.children.find(s => s.type === 'text' && s.characters === '03 / 无更新：Android 系统 Toast');
if (caption) { caption.characters = '03 / 手动无更新：系统 Toast'; caption.name = caption.characters; }
return { id: note.id, name: note.name, showInViewMode: note.showInViewMode,
  bounds: { x: note.x, y: note.y, width: note.width, height: note.height },
  text: note.children.filter(s => s.type === 'text').map(s => s.characters) };
