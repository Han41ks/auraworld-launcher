'use strict';
/* global $, $$, esc, cleanErr, S, aw, skinview3d */

// Части развёртки скина 64×64 (x, y, w, h, подпись, слой одежды?)
const SKIN_PARTS = [
	[0, 0, 32, 16, 'Голова', false], [32, 0, 32, 16, 'Голова — шапка', true],
	[16, 16, 24, 16, 'Тело', false], [16, 32, 24, 16, 'Тело — куртка', true],
	[40, 16, 16, 16, 'Правая рука', false], [40, 32, 16, 16, 'Правая рука — рукав', true],
	[0, 16, 16, 16, 'Правая нога', false], [0, 32, 16, 16, 'Правая нога — штанина', true],
	[32, 48, 16, 16, 'Левая рука', false], [48, 48, 16, 16, 'Левая рука — рукав', true],
	[16, 48, 16, 16, 'Левая нога', false], [0, 48, 16, 16, 'Левая нога — штанина', true],
];
const PALETTE = ['#000000', '#3b2b20', '#6b4a32', '#a87452', '#d9a066', '#f1c9a5', '#ffffff', '#9ba4ae',
	'#19c3b0', '#0e8f84', '#2563eb', '#7c3aed', '#db2777', '#dc2626', '#f59e0b', '#16a34a'];
const SCALE = 8;

const SK = {
	viewer: null, tex: null, ctx: null, view: null, vctx: null,
	tool: 'brush', color: '#19c3b0', drawing: false, undo: [], redo: [],
	editing: { id: null, name: '', variant: 'classic' }, list: { active: null, skins: [] },
	guides: true, dirty: false, lastPreview: 0,
	layer: 'base', paint3d: false,
};

// ---------------- Инициализация ----------------
function skinInit() {
	if (SK.tex) return;
	SK.tex = document.createElement('canvas');
	SK.tex.width = 64; SK.tex.height = 64;
	SK.ctx = SK.tex.getContext('2d', { willReadFrequently: true });
	SK.view = $('#skin-canvas');
	SK.vctx = SK.view.getContext('2d');
	SK.vctx.imageSmoothingEnabled = false;

	SK.viewer = new skinview3d.SkinViewer({ canvas: $('#skin-3d'), width: 400, height: 460 });
	SK.viewer.controls.enableZoom = true;
	SK.viewer.controls.enablePan = false;
	SK.viewer.autoRotate = false;
	SK.viewer.animation = new skinview3d.WalkingAnimation();
	SK.viewer.animation.speed = 0.6;
	SK.viewer.animation.paused = true;
	SK.viewer.zoom = 0.9;
	init3dPainting();
	window.addEventListener('resize', fit3d);
	$$('.layer-pick .tab').forEach((t) => t.addEventListener('click', () => setLayer(t.dataset.layer)));

	$('#palette').innerHTML = PALETTE.map((c) => `<i style="background:${c}" data-c="${c}"></i>`).join('');
	$$('#palette i').forEach((i) => i.addEventListener('click', () => setColor(i.dataset.c)));
	$('#skin-color').addEventListener('input', (e) => setColor(e.target.value));
	$$('.tool[data-tool]').forEach((b) => b.addEventListener('click', () => setTool(b.dataset.tool)));
	$('#skin-undo').onclick = undo;
	$('#skin-redo').onclick = redoAct;
	$('#skin-guides').addEventListener('change', (e) => { SK.guides = e.target.checked; redraw(); });
	$('#skin-walk').addEventListener('change', (e) => { SK.viewer.animation.paused = !e.target.checked; if (!e.target.checked) SK.viewer.playerObject.resetJoints && SK.viewer.playerObject.resetJoints(); });
	$$('.variant-pick .tab').forEach((t) => t.addEventListener('click', () => { SK.editing.variant = t.dataset.variant; SK.dirty = true; syncVariant(); preview(true); }));

	SK.view.addEventListener('mousedown', (e) => { SK.drawing = true; pushUndo(); applyTool(e); });
	SK.view.addEventListener('mousemove', (e) => { showHint(e); if (SK.drawing && (SK.tool === 'brush' || SK.tool === 'eraser')) applyTool(e); });
	window.addEventListener('mouseup', () => { if (SK.drawing) { SK.drawing = false; preview(true); } });
	SK.view.addEventListener('mouseleave', () => { $('#part-hint').textContent = ''; });
	SK.view.addEventListener('contextmenu', (e) => { e.preventDefault(); pick(e); });

	document.addEventListener('keydown', (e) => {
		if (!$('#page-accounts').classList.contains('active') || e.target.tagName === 'INPUT') return;
		if (e.ctrlKey && e.key.toLowerCase() === 'z') { e.preventDefault(); undo(); }
		else if (e.ctrlKey && e.key.toLowerCase() === 'y') { e.preventDefault(); redoAct(); }
		else if (!e.ctrlKey) ({ b: 'brush', e: 'eraser', f: 'fill', i: 'picker' }[e.key.toLowerCase()] && setTool({ b: 'brush', e: 'eraser', f: 'fill', i: 'picker' }[e.key.toLowerCase()]));
	});

	$('#skin-new').onclick = (e) => { e.stopPropagation(); $('#skin-new-menu').classList.toggle('hidden'); };
	document.addEventListener('click', () => $('#skin-new-menu').classList.add('hidden'));
	$$('[data-new]').forEach((b) => b.addEventListener('click', () => newSkin(b.dataset.new)));
	$('#skin-save').onclick = () => saveSkin(false);
	$('#skin-wear').onclick = () => saveSkin(true);
	$('#skin-export').onclick = () => aw.skinsExport(SK.tex.toDataURL('image/png'), SK.editing.name || 'skin');
	$('#skin-delete').onclick = deleteSkin;
	$('#skin-name').addEventListener('input', (e) => { SK.editing.name = e.target.value; SK.dirty = true; });
	$('#skin-server-name').textContent = S.config.serverName;
}

async function openSkinPage() {
	skinInit();
	requestAnimationFrame(fit3d);
	SK.list = await aw.skinsList();
	renderWardrobe();
	if (!SK.editing.id && !SK.dirty) {
		const first = SK.list.skins.find((s) => s.id === SK.list.active) || SK.list.skins[0];
		if (first) await editSkin(first);
		else await newSkin('steve', true);
	}
}

// ---------------- Гардероб ----------------
function faceCanvas(dataUrl) {
	const c = document.createElement('canvas');
	c.width = 8; c.height = 8;
	const img = new Image();
	img.onload = () => {
		const x = c.getContext('2d');
		x.drawImage(img, 8, 8, 8, 8, 0, 0, 8, 8);
		x.drawImage(img, 40, 8, 8, 8, 0, 0, 8, 8);
	};
	img.src = dataUrl;
	return c;
}

function renderWardrobe() {
	const box = $('#skin-list');
	box.innerHTML = '';
	if (!SK.list.skins.length) box.innerHTML = '<div class="muted small">Пока пусто — нарисуй скин и нажми «Сохранить».</div>';
	for (const s of SK.list.skins) {
		const el = document.createElement('div');
		el.className = 'skin-item' + (s.id === SK.editing.id ? ' editing' : '');
		el.appendChild(faceCanvas(s.data));
		el.insertAdjacentHTML('beforeend', `<div class="nm">${esc(s.name)}</div>${s.id === SK.list.active ? '<div class="worn">✓ надет</div>' : ''}`);
		el.onclick = () => editSkin(s);
		box.appendChild(el);
	}
}

function loadTexture(dataUrl) {
	return new Promise((resolve, reject) => {
		const img = new Image();
		img.onload = () => {
			SK.ctx.clearRect(0, 0, 64, 64);
			SK.ctx.drawImage(img, 0, 0);
			if (img.height === 32) {
				// Старый формат 64×32: левые рука и нога = копия правых
				SK.ctx.drawImage(img, 0, 16, 16, 16, 16, 48, 16, 16);
				SK.ctx.drawImage(img, 40, 16, 16, 16, 32, 48, 16, 16);
			}
			resolve();
		};
		img.onerror = () => reject(new Error('Не удалось открыть картинку'));
		img.src = dataUrl;
	});
}

async function editSkin(s) {
	if (SK.dirty && !confirm('Есть несохранённые изменения. Открыть другой скин?')) return;
	await loadTexture(s.data);
	SK.editing = { id: s.id, name: s.name, variant: s.variant };
	afterLoad();
}

async function newSkin(kind, silent) {
	$('#skin-new-menu').classList.add('hidden');
	if (!silent && SK.dirty && !confirm('Есть несохранённые изменения. Начать новый скин?')) return;
	try {
		let name = 'Мой скин', variant = 'classic';
		if (kind === 'steve' || kind === 'alex') {
			const t = await aw.skinsTemplates();
			const tpl = t.find((x) => (kind === 'alex' ? x.variant === 'slim' : x.variant === 'classic'));
			if (tpl) { await loadTexture(tpl.dataUrl); variant = tpl.variant; }
			else { blankTexture(); if (!silent) status('Шаблоны появятся после первого запуска игры — пока чистый лист.'); }
			if (kind === 'alex') variant = 'slim';
		} else if (kind === 'blank') {
			blankTexture();
		} else if (kind === 'file') {
			const d = await aw.skinsPickFile();
			if (!d) return;
			await loadTexture(d);
			name = 'Загруженный скин';
		} else if (kind === 'licensed') {
			const nick = await askNick();
			if (!nick) return;
			status('Ищу скин...');
			const r = await aw.skinsFromLicensed(nick);
			await loadTexture(r.dataUrl);
			name = `Скин ${r.name}`;
			variant = r.variant;
		}
		SK.editing = { id: null, name, variant };
		afterLoad();
		SK.dirty = !silent;
		if (!silent) status('Новый скин — не забудь сохранить.');
	} catch (e) {
		status(cleanErr(e), true);
	}
}

function askNick() {
	return new Promise((resolve) => {
		const box = $('#skin-status');
		box.innerHTML = '<div class="search-row"><input id="lic-nick" maxlength="16" placeholder="Ник лицензии" spellcheck="false"><button class="btn-small btn-accent" id="lic-ok">Найти</button></div>';
		const input = $('#lic-nick');
		input.focus();
		const done = (v) => { box.innerHTML = ''; resolve(v); };
		$('#lic-ok').onclick = () => done(input.value.trim());
		input.addEventListener('keydown', (e) => { if (e.key === 'Enter') done(input.value.trim()); if (e.key === 'Escape') done(null); });
	});
}

function blankTexture() {
	SK.ctx.clearRect(0, 0, 64, 64);
	SK.ctx.fillStyle = '#9ba4ae';
	for (const [x, y, w, h, , overlay] of SKIN_PARTS) if (!overlay) SK.ctx.fillRect(x, y, w, h);
}

function afterLoad() {
	SK.undo = []; SK.redo = []; SK.dirty = false;
	$('#skin-name').value = SK.editing.name;
	syncVariant();
	redraw();
	preview(true);
	renderWardrobe();
	status('');
}

function syncVariant() {
	$$('.variant-pick .tab').forEach((t) => t.classList.toggle('active', t.dataset.variant === SK.editing.variant));
}

async function saveSkin(wear) {
	try {
		const r = await aw.skinsSave({ id: SK.editing.id, name: SK.editing.name || 'Мой скин', variant: SK.editing.variant, dataUrl: SK.tex.toDataURL('image/png') });
		SK.editing.id = r.id;
		SK.dirty = false;
		SK.list = wear ? await aw.skinsSetActive(r.id) : r;
		renderWardrobe();
		status(wear ? '✓ Скин надет — применится при следующем входе на сервер.' : '✓ Сохранено');
		if (typeof renderAccountChip === 'function') renderAccountChip();
	} catch (e) {
		status(cleanErr(e), true);
	}
}

async function deleteSkin() {
	if (!SK.editing.id) { await newSkin('steve', true); return; }
	if (!confirm(`Удалить скин «${SK.editing.name}»?`)) return;
	SK.list = await aw.skinsRemove(SK.editing.id);
	SK.editing = { id: null, name: '', variant: 'classic' };
	SK.dirty = false;
	const next = SK.list.skins[0];
	if (next) await editSkin(next); else await newSkin('steve', true);
}

function status(text, err) {
	$('#skin-status').innerHTML = text ? `<span style="color:${err ? '#ffb3ae' : 'var(--accent)'}">${esc(text)}</span>` : '';
}

// ---------------- Рисование ----------------
function setTool(t) {
	SK.tool = t;
	$$('.tool[data-tool]').forEach((b) => b.classList.toggle('active', b.dataset.tool === t));
}
function setColor(c) {
	SK.color = c;
	$('#skin-color').value = c;
	if (SK.tool === 'eraser' || SK.tool === 'picker') setTool('brush');
}

function cell(e) {
	const r = SK.view.getBoundingClientRect();
	return { x: Math.floor(((e.clientX - r.left) / r.width) * 64), y: Math.floor(((e.clientY - r.top) / r.height) * 64) };
}

function partAt(x, y) {
	return SKIN_PARTS.find(([px, py, w, h]) => x >= px && x < px + w && y >= py && y < py + h);
}

function showHint(e) {
	const { x, y } = cell(e);
	const p = partAt(x, y);
	$('#part-hint').textContent = p ? `${p[4]}  (${x}, ${y})` : '';
}

function hexToRgba(hex) {
	const n = parseInt(hex.slice(1), 16);
	return [(n >> 16) & 255, (n >> 8) & 255, n & 255, 255];
}

function applyTool(e) {
	const { x, y } = cell(e);
	applyPixel(x, y);
}

/** Применить текущий инструмент к пикселю текстуры (общая логика для развёртки и 3D). */
function applyPixel(x, y, from3d) {
	if (x < 0 || y < 0 || x > 63 || y > 63 || !partAt(x, y)) return;
	if (SK.tool === 'picker') return pickPixel(x, y);
	if (SK.tool === 'fill') { flood(x, y, hexToRgba(SK.color)); redraw(); preview(true); return; }
	if (SK.tool === 'eraser') SK.ctx.clearRect(x, y, 1, 1);
	else { SK.ctx.fillStyle = SK.color; SK.ctx.fillRect(x, y, 1, 1); }
	SK.dirty = true;
	redraw();
	from3d ? preview3dFast() : preview(false);
}

function pick(e) {
	const { x, y } = cell(e);
	pickPixel(x, y);
}

function pickPixel(x, y) {
	const d = SK.ctx.getImageData(x, y, 1, 1).data;
	if (d[3] === 0) return;
	setColor('#' + [d[0], d[1], d[2]].map((v) => v.toString(16).padStart(2, '0')).join(''));
}

/** Заливка в пределах одной части скина. */
function flood(sx, sy, rgba) {
	const part = partAt(sx, sy);
	const img = SK.ctx.getImageData(0, 0, 64, 64);
	const d = img.data;
	const at = (x, y) => (y * 64 + x) * 4;
	const t = d.slice(at(sx, sy), at(sx, sy) + 4);
	if (t[0] === rgba[0] && t[1] === rgba[1] && t[2] === rgba[2] && t[3] === rgba[3]) return;
	const same = (i) => d[i] === t[0] && d[i + 1] === t[1] && d[i + 2] === t[2] && d[i + 3] === t[3];
	const [px, py, pw, ph] = part;
	const stack = [[sx, sy]];
	while (stack.length) {
		const [x, y] = stack.pop();
		if (x < px || y < py || x >= px + pw || y >= py + ph) continue;
		const i = at(x, y);
		if (!same(i)) continue;
		d[i] = rgba[0]; d[i + 1] = rgba[1]; d[i + 2] = rgba[2]; d[i + 3] = rgba[3];
		stack.push([x + 1, y], [x - 1, y], [x, y + 1], [x, y - 1]);
	}
	SK.ctx.putImageData(img, 0, 0);
	SK.dirty = true;
}

function pushUndo() {
	SK.undo.push(SK.ctx.getImageData(0, 0, 64, 64));
	if (SK.undo.length > 60) SK.undo.shift();
	SK.redo = [];
}
function undo() {
	if (!SK.undo.length) return;
	SK.redo.push(SK.ctx.getImageData(0, 0, 64, 64));
	SK.ctx.putImageData(SK.undo.pop(), 0, 0);
	SK.dirty = true; redraw(); preview(true);
}
function redoAct() {
	if (!SK.redo.length) return;
	SK.undo.push(SK.ctx.getImageData(0, 0, 64, 64));
	SK.ctx.putImageData(SK.redo.pop(), 0, 0);
	SK.dirty = true; redraw(); preview(true);
}


// ---------------- Рисование по 3D-модели ----------------
const PART_NAMES = { head: 'Голова', body: 'Тело', rightArm: 'Правая рука', leftArm: 'Левая рука', rightLeg: 'Правая нога', leftLeg: 'Левая нога' };

function fit3d() {
	const wrap = $('.skin-3d-wrap');
	if (!SK.viewer || !wrap || !wrap.clientWidth) return;
	SK.viewer.setSize(wrap.clientWidth, 460);
}

function setLayer(layer) {
	SK.layer = layer;
	$$('.layer-pick .tab').forEach((t) => t.classList.toggle('active', t.dataset.layer === layer));
	// «Основа»: второй слой прячем, чтобы он не мешал рисовать по коже
	SK.viewer.playerObject.skin.setOuterLayerVisible(layer === 'outer');
	$('.hint-3d-help').textContent = layer === 'outer'
		? 'Рисуешь второй слой: шапку, куртку, рукава, штанины'
		: 'Второй слой скрыт, чтобы рисовать по основе · тяни мимо модели — повернуть · колесо — приблизить';
}

/** Луч из камеры через точку курсора → ближайшее попадание в модель → пиксель текстуры. */
function raycast3d(e) {
	const canvas = $('#skin-3d');
	const r = canvas.getBoundingClientRect();
	const cam = SK.viewer.camera;
	const V3 = cam.position.constructor;
	const M4 = cam.matrixWorld.constructor;
	const ndcX = ((e.clientX - r.left) / r.width) * 2 - 1;
	const ndcY = -((e.clientY - r.top) / r.height) * 2 + 1;
	const origin = new V3().setFromMatrixPosition(cam.matrixWorld);
	const far = new V3(ndcX, ndcY, 0.5).unproject(cam);

	const skin = SK.viewer.playerObject.skin;
	let best = null;
	for (const key of Object.keys(PART_NAMES)) {
		const part = skin[key];
		const mesh = SK.layer === 'outer' ? part.outerLayer : part.innerLayer;
		if (!mesh || !mesh.geometry) continue;
		const inv = new M4().copy(mesh.matrixWorld).invert();
		const o = origin.clone().applyMatrix4(inv);
		const p = far.clone().applyMatrix4(inv);
		const d = [p.x - o.x, p.y - o.y, p.z - o.z];
		const pos = mesh.geometry.attributes.position.array;
		const uv = mesh.geometry.attributes.uv.array;
		const idx = mesh.geometry.index ? mesh.geometry.index.array : null;
		const count = idx ? idx.length : pos.length / 3;
		for (let i = 0; i < count; i += 3) {
			const a = idx ? idx[i] : i, b = idx ? idx[i + 1] : i + 1, c = idx ? idx[i + 2] : i + 2;
			const hit = triangleHit(o, d, pos, a, b, c);
			if (!hit) continue;
			const world = new V3(o.x + d[0] * hit.t, o.y + d[1] * hit.t, o.z + d[2] * hit.t).applyMatrix4(mesh.matrixWorld);
			const dist = world.distanceTo(origin);
			if (best && dist >= best.dist) continue;
			const w = 1 - hit.u - hit.v;
			const U = uv[a * 2] * w + uv[b * 2] * hit.u + uv[c * 2] * hit.v;
			const V = uv[a * 2 + 1] * w + uv[b * 2 + 1] * hit.u + uv[c * 2 + 1] * hit.v;
			// Границы грани в пикселях — чтобы попадание на самый край не «уехало» на соседнюю грань
			const us = [uv[a * 2], uv[b * 2], uv[c * 2]].map((x) => x * 64);
			const vs = [uv[a * 2 + 1], uv[b * 2 + 1], uv[c * 2 + 1]].map((y) => (1 - y) * 64);
			const clamp = (val, arr) => Math.min(Math.max(Math.floor(val), Math.round(Math.min(...arr))), Math.round(Math.max(...arr)) - 1);
			best = { dist, part: key, x: clamp(U * 64, us), y: clamp((1 - V) * 64, vs) };
		}
	}
	return best;
}

/** Пересечение луча с треугольником (Мёллер — Трумбор). */
function triangleHit(o, d, pos, a, b, c) {
	const ax = pos[a * 3], ay = pos[a * 3 + 1], az = pos[a * 3 + 2];
	const e1 = [pos[b * 3] - ax, pos[b * 3 + 1] - ay, pos[b * 3 + 2] - az];
	const e2 = [pos[c * 3] - ax, pos[c * 3 + 1] - ay, pos[c * 3 + 2] - az];
	const pv = [d[1] * e2[2] - d[2] * e2[1], d[2] * e2[0] - d[0] * e2[2], d[0] * e2[1] - d[1] * e2[0]];
	const det = e1[0] * pv[0] + e1[1] * pv[1] + e1[2] * pv[2];
	if (Math.abs(det) < 1e-9) return null;
	const inv = 1 / det;
	const tv = [o.x - ax, o.y - ay, o.z - az];
	const u = (tv[0] * pv[0] + tv[1] * pv[1] + tv[2] * pv[2]) * inv;
	if (u < 0 || u > 1) return null;
	const qv = [tv[1] * e1[2] - tv[2] * e1[1], tv[2] * e1[0] - tv[0] * e1[2], tv[0] * e1[1] - tv[1] * e1[0]];
	const v = (d[0] * qv[0] + d[1] * qv[1] + d[2] * qv[2]) * inv;
	if (v < 0 || u + v > 1) return null;
	const t = (e2[0] * qv[0] + e2[1] * qv[1] + e2[2] * qv[2]) * inv;
	return t > 0 ? { t, u, v } : null;
}

let last3d = 0;
function preview3dFast() {
	const now = performance.now();
	if (now - last3d < 30) return;
	last3d = now;
	SK.viewer.loadSkin(SK.tex, { model: SK.editing.variant === 'slim' ? 'slim' : 'default' });
}

function init3dPainting() {
	const canvas = $('#skin-3d');
	// capture: перехватываем нажатие раньше вращения камеры — если попали в модель, рисуем
	canvas.addEventListener('pointerdown', (e) => {
		const hit = raycast3d(e);
		if (!hit) return; // мимо модели — пусть камера вращается
		e.stopImmediatePropagation();
		e.preventDefault();
		if (e.button === 2) { pickPixel(hit.x, hit.y); return; }
		if (e.button !== 0) return;
		SK.paint3d = true;
		canvas.setPointerCapture(e.pointerId);
		pushUndo();
		applyPixel(hit.x, hit.y, true);
	}, { capture: true });
	canvas.addEventListener('pointermove', (e) => {
		const hit = raycast3d(e);
		$('#hint-3d').textContent = hit ? `${PART_NAMES[hit.part]}${SK.layer === 'outer' ? ' — второй слой' : ''}` : '';
		canvas.style.cursor = hit ? 'crosshair' : 'grab';
		if (SK.paint3d && hit && (SK.tool === 'brush' || SK.tool === 'eraser')) applyPixel(hit.x, hit.y, true);
	});
	const end = () => { if (SK.paint3d) { SK.paint3d = false; preview(true); } };
	canvas.addEventListener('pointerup', end);
	canvas.addEventListener('pointercancel', end);
	canvas.addEventListener('pointerleave', () => { $('#hint-3d').textContent = ''; });
	canvas.addEventListener('contextmenu', (e) => e.preventDefault());
}

// ---------------- Отрисовка ----------------
function redraw() {
	const v = SK.vctx;
	v.clearRect(0, 0, 512, 512);
	v.imageSmoothingEnabled = false;
	v.drawImage(SK.tex, 0, 0, 512, 512);
	if (!SK.guides) return;
	for (const [x, y, w, h, , overlay] of SKIN_PARTS) {
		v.strokeStyle = overlay ? 'rgba(124,58,237,0.55)' : 'rgba(25,195,176,0.55)';
		v.setLineDash(overlay ? [4, 3] : []);
		v.lineWidth = 1;
		v.strokeRect(x * SCALE + 0.5, y * SCALE + 0.5, w * SCALE - 1, h * SCALE - 1);
	}
	v.setLineDash([]);
	v.strokeStyle = 'rgba(255,255,255,0.05)';
	v.beginPath();
	for (let i = 1; i < 64; i++) { v.moveTo(i * SCALE + 0.5, 0); v.lineTo(i * SCALE + 0.5, 512); v.moveTo(0, i * SCALE + 0.5); v.lineTo(512, i * SCALE + 0.5); }
	v.stroke();
}

function preview(force) {
	const now = Date.now();
	if (!force && now - SK.lastPreview < 120) return;
	SK.lastPreview = now;
	SK.viewer.loadSkin(SK.tex, { model: SK.editing.variant === 'slim' ? 'slim' : 'default' });
	SK.viewer.playerObject.skin.setOuterLayerVisible(SK.layer === 'outer');
}
