'use strict';
const $ = (s) => document.querySelector(s);
const $$ = (s) => [...document.querySelectorAll(s)];
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const cleanErr = (e) => {
	let m = String((e && e.message) || e).replace(/^Error invoking remote method '[^']+': (\w*Error: )?/, '');
	if (/fetch failed|ENOTFOUND|ECONNRESET|ETIMEDOUT|network/i.test(m)) m = 'Нет связи с сервером загрузки. Проверь интернет (или антивирус/фаервол) и попробуй ещё раз.';
	return m;
};
const fmtNum = (n) => (n >= 1e6 ? (n / 1e6).toFixed(1).replace('.0', '') + ' млн' : n >= 1e3 ? Math.round(n / 1e3) + ' тыс.' : String(n));
const fmtDate = (d) => {
	const days = Math.floor((Date.now() - new Date(d)) / 864e5);
	if (days < 1) return 'сегодня';
	if (days < 30) return `${days} дн. назад`;
	if (days < 365) return `${Math.floor(days / 30)} мес. назад`;
	return `${Math.floor(days / 365)} г. назад`;
};
const FALLBACK_HEAD = 'data:image/svg+xml;utf8,' + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><rect width="8" height="8" fill="#2c4a52"/><rect x="1" y="3" width="2" height="1" fill="#e6edf3"/><rect x="5" y="3" width="2" height="1" fill="#e6edf3"/><rect x="2" y="5" width="4" height="1" fill="#19c3b0"/></svg>');
function fixAvatars() { $$('img.avatar').forEach((i) => i.addEventListener('error', () => { i.src = FALLBACK_HEAD; }, { once: true })); }
const head = (name, size = 32) => `https://mc-heads.net/avatar/${encodeURIComponent(name)}/${size}`;

const S = {
	config: null, settings: null, instances: [], selected: null, manifest: null, playing: false,
	cat: { type: 'mod', query: '', sort: 'relevance', offset: 0, categories: new Set(), target: null },
};

// ================= Навигация =================
function go(page) {
	$$('.nav').forEach((x) => x.classList.toggle('active', x.dataset.page === page));
	$$('.page').forEach((p) => p.classList.toggle('active', p.id === 'page-' + page));
	if (page === 'builds') renderBuilds();
	if (page === 'catalog') openCatalog();
	if (page === 'java') renderJava();

	if (page === 'accounts') { renderAccounts(); openSkinPage(); }
}
$$('.nav').forEach((b) => b.addEventListener('click', () => go(b.dataset.page)));
$$('[data-close]').forEach((b) => b.addEventListener('click', () => b.closest('.modal').classList.add('hidden')));
$$('.modal:not(#modal-guide)').forEach((m) => m.addEventListener('mousedown', (e) => e.target === m && m.classList.add('hidden')));

// ================= Старт =================
async function init() {
	S.config = await aw.config();
	S.settings = await aw.settings();
	$('#server-name').textContent = S.config.serverName;
	$('#hero-title').textContent = S.config.serverName;
	$('#hero-sub').textContent = `Сервер на Minecraft ${S.config.mcVersion} · Fabric`;
	$('#launcher-version').textContent = `Лаунчер v${S.config.version}`;
	renderSettings();
	renderAccountChip();
	await reloadInstances();
	aw.manifest().then((m) => { S.manifest = m; }).catch(() => {});
}

async function reloadInstances() {
	S.instances = await aw.instances();
	if (!S.selected || !S.instances.find((i) => i.id === S.selected)) S.selected = S.settings.lastInstance;
	if (!S.instances.find((i) => i.id === S.selected)) S.selected = S.instances[0].id;
	const opts = S.instances.map((i) => `<option value="${esc(i.id)}">${esc(i.name)} · ${esc(i.mcVersion)}${i.loader === 'vanilla' ? '' : ' Fabric'}</option>`).join('');
	$('#play-instance').innerHTML = opts;
	$('#play-instance').value = S.instances.find((i) => i.id === S.settings.lastInstance) ? S.settings.lastInstance : S.instances[0].id;
	$('#catalog-target').innerHTML = opts;
	renderAutoJoin();
	$('#catalog-target').value = S.cat.target && S.instances.find((i) => i.id === S.cat.target) ? S.cat.target : S.selected;
}

// ================= Играть =================
function activeAccount() {
	return (S.settings.accounts || []).find((a) => a.active);
}

function renderAccountChip() {
	const a = activeAccount();
	$('#account-chip').innerHTML = a
		? `<img class="avatar" src="${head(a.name)}" alt=""><div><div class="name">${esc(a.name)}</div><div class="muted small">${a.type === 'msa' ? 'Лицензия Microsoft' : 'Вход по нику'}</div></div><span class="link" id="chip-change">сменить</span>`
		: `<div class="muted">Нет аккаунта</div><span class="link" id="chip-change">добавить</span>`;
	$('#chip-change').onclick = () => go('accounts');
	fixAvatars();
}

$('#play-btn').addEventListener('click', async () => {
	if (S.playing) return;
	if (!activeAccount()) return go('accounts');
	$('#play-error').classList.add('hidden');
	$('#progress-wrap').classList.remove('hidden');
	$('#game-log').textContent = '';
	setPlaying(true);
	try {
		await aw.play($('#play-instance').value);
	} catch (e) {
		setPlaying(false);
		$('#progress-wrap').classList.add('hidden');
		$('#play-error').textContent = cleanErr(e);
		$('#play-error').classList.remove('hidden');
	}
});

function setPlaying(on) {
	S.playing = on;
	$('#play-btn').disabled = on;
	$('#play-btn').textContent = on ? 'ЗАПУСК...' : 'ИГРАТЬ';
}
aw.on('progress', ({ text, fraction }) => {
	$('#progress-text').textContent = text;
	if (typeof fraction === 'number') $('#progress-bar').style.width = Math.round(fraction * 100) + '%';
});
aw.on('game:started', () => { $('#play-btn').textContent = 'В ИГРЕ'; $('#progress-text').textContent = 'Игра запущена'; });
aw.on('game:exit', (code) => {
	setPlaying(false);
	$('#progress-wrap').classList.add('hidden');
	if (code) {
		$('#play-error').textContent = `Игра закрылась с ошибкой (код ${code}). Подробности — в «Журнале игры».`;
		$('#play-error').classList.remove('hidden');
	}
});
aw.on('gamelog', (line) => {
	const pre = $('#game-log');
	pre.textContent += line.endsWith('\n') ? line : line + '\n';
	if (pre.textContent.length > 200000) pre.textContent = pre.textContent.slice(-150000);
	pre.scrollTop = pre.scrollHeight;
});

// Галочка «Сразу зайти на сервер»
function renderAutoJoin() {
	const inst = S.instances.find((i) => i.id === $('#play-instance').value);
	const compatible = inst && inst.loader === 'fabric' && inst.mcVersion === S.config.mcVersion;
	const hasAddress = Boolean(S.config.serverAddress);
	const box = $('#autojoin');
	$('#autojoin-name').textContent = S.config.serverName;
	box.checked = S.settings.autoJoin !== false;
	box.disabled = !compatible || !hasAddress;
	$('#autojoin-wrap').classList.toggle('disabled', box.disabled);
	$('#autojoin-hint').textContent = !hasAddress
		? '— адрес сервера ещё не указан в настройках лаунчера'
		: !compatible ? '— эта сборка не подходит для сервера' : '';
}
$('#autojoin').addEventListener('change', async (e) => {
	S.settings = await aw.saveSettings({ autoJoin: e.target.checked });
});
$('#play-instance').addEventListener('change', renderAutoJoin);

// ================= Сборки =================
const KEYS = [['mods', 'Моды'], ['resourcepacks', 'Ресурспаки'], ['shaders', 'Шейдеры']];

function renderBuilds() {
	$('#builds').innerHTML = S.instances
		.map((i) => `<div class="build-item ${i.id === S.selected ? 'active' : ''}" data-id="${esc(i.id)}">
			${esc(i.name)}${i.official ? '<span class="badge">сервер</span>' : ''}
			<div class="muted small">${esc(i.mcVersion)} · ${i.loader === 'vanilla' ? 'без модов' : 'Fabric'}</div></div>`)
		.join('');
	$$('.build-item').forEach((el) => el.addEventListener('click', () => { S.selected = el.dataset.id; renderBuilds(); }));
	renderBuildDetails();
}

function renderBuildDetails() {
	const inst = S.instances.find((i) => i.id === S.selected);
	if (!inst) return;
	const m = S.manifest || { mandatory: [], officialPack: [] };
	const serverOk = inst.loader === 'fabric' && inst.mcVersion === S.config.mcVersion;
	const locked = (names, label) => names.map((n) => `<div class="mod-row"><span>${esc(n)}</span><span class="badge lock">🔒 ${label}</span></div>`).join('');
	const content = KEYS.map(([key, title]) => {
		const items = inst[key] || [];
		if (key === 'mods' && inst.loader === 'vanilla') return '';
		return `<div class="section-title">${title}</div>` + (items.length
			? items.map((x) => `<div class="mod-row"><span>${esc(x.title)} <span class="muted small">${esc(x.version || '')}</span>${x.dependency ? '<span class="badge lock">зависимость</span>' : ''}</span>
				<button class="btn-small btn-danger" data-remove="${esc(x.projectId)}" data-key="${key}">Убрать</button></div>`).join('')
			: '<div class="muted small">Пусто</div>');
	}).join('');

	$('#build-details').innerHTML = `
		<div class="row-between">
			<div><h2>${esc(inst.name)}</h2><div class="muted small">Minecraft ${esc(inst.mcVersion)} · ${inst.loader === 'vanilla' ? 'без модов' : 'Fabric ' + esc(inst.loaderVersion || '')}</div></div>
			<div>
				<button class="btn-small btn-accent" id="to-catalog">+ Добавить из каталога</button>
				<button class="btn-small" id="migrate-btn">↗ Другая версия</button>
				<button class="btn-small" id="open-folder">Папка</button>
				${inst.official ? '' : '<button class="btn-small btn-danger" id="del-build">Удалить</button>'}
			</div>
		</div>
		${serverOk ? `<div class="section-title">Моды сервера (ставятся всегда)</div>${locked(m.mandatory, 'обязательно')}` : `<div class="muted small" style="margin-top:12px">Эта сборка не на версии сервера (${esc(S.config.mcVersion)} + Fabric), поэтому на сервер AuraWorld с ней не зайти.</div>`}
		${inst.official ? `<div class="section-title">Официальная сборка</div>${locked(m.officialPack, 'в сборке')}` : ''}
		${content}`;

	$('#open-folder').onclick = () => aw.openInstance(inst.id);
	$('#migrate-btn').onclick = () => openMigrate(inst);
	$('#to-catalog').onclick = () => { S.cat.target = inst.id; go('catalog'); };
	const del = $('#del-build');
	if (del) del.onclick = async () => {
		if (!confirm(`Удалить сборку «${inst.name}» вместе с её мирами и настройками?`)) return;
		await aw.removeInstance(inst.id);
		S.selected = null;
		await reloadInstances();
		renderBuilds();
	};
	$$('[data-remove]').forEach((b) => b.addEventListener('click', async () => {
		await aw.removeContent(inst.id, b.dataset.key, b.dataset.remove);
		await reloadInstances();
		renderBuilds();
	}));
}

// ---- Новая сборка ----
let versionList = [];
async function fillVersions() {
	$('#nb-version').innerHTML = '<option>Загрузка...</option>';
	try {
		versionList = await aw.versions($('#nb-snapshots').checked);
		$('#nb-version').innerHTML = versionList.map((v) => `<option value="${esc(v.id)}">${esc(v.id)}${v.type === 'snapshot' ? ' (снапшот)' : ''}</option>`).join('');
		$('#nb-version').value = versionList.find((v) => v.id === S.config.mcVersion) ? S.config.mcVersion : versionList[0].id;
		updateLoaderNote();
	} catch (e) {
		$('#nb-version').innerHTML = '';
		$('#nb-error').textContent = cleanErr(e);
		$('#nb-error').classList.remove('hidden');
	}
}
function updateLoaderNote() {
	const v = versionList.find((x) => x.id === $('#nb-version').value);
	const fab = $('input[name="nb-loader"][value="fabric"]');
	fab.disabled = !v || !v.fabric;
	if (fab.disabled) $('input[name="nb-loader"][value="vanilla"]').checked = true;
	$('#nb-note').textContent = v && !v.fabric ? 'Для этой версии нет Fabric — только чистый Minecraft.' : 'Java нужной версии скачается сама при первом запуске.';
}
$('#new-build').addEventListener('click', () => {
	$('#nb-error').classList.add('hidden');
	$('#modal-new').classList.remove('hidden');
	fillVersions();
});
$('#nb-snapshots').addEventListener('change', fillVersions);
$('#nb-version').addEventListener('change', updateLoaderNote);
$('#nb-create').addEventListener('click', async () => {
	const btn = $('#nb-create');
	btn.disabled = true;
	try {
		const inst = await aw.createInstance({
			name: $('#nb-name').value.trim() || 'Моя сборка',
			mcVersion: $('#nb-version').value,
			loader: $('input[name="nb-loader"]:checked').value,
		});
		$('#modal-new').classList.add('hidden');
		S.selected = inst.id;
		await reloadInstances();
		renderBuilds();
	} catch (e) {
		$('#nb-error').textContent = cleanErr(e);
		$('#nb-error').classList.remove('hidden');
	} finally {
		btn.disabled = false;
	}
});


// ================= Перенос сборки на другую версию =================
let mgInst = null;
let mgPlan = null;

async function openMigrate(inst) {
	mgInst = inst;
	mgPlan = null;
	$('#mg-title').textContent = `Перенос «${inst.name}» (сейчас ${inst.mcVersion})`;
	$('#mg-results').innerHTML = '';
	$('#mg-apply').classList.add('hidden');
	$('#mg-worlds-wrap').classList.add('hidden');
	$('#mg-progress').classList.add('hidden');
	$('#mg-status').textContent = 'Выбери версию — лаунчер проверит, какие моды для неё есть.';
	$('#modal-migrate').classList.remove('hidden');
	$('#mg-version').innerHTML = '<option>Загрузка...</option>';
	try {
		const vs = await aw.versions(false);
		$('#mg-version').innerHTML = vs.filter((v) => v.id !== inst.mcVersion && (inst.loader === 'vanilla' || v.fabric))
			.map((v) => `<option value="${esc(v.id)}">${esc(v.id)}</option>`).join('');
	} catch (e) {
		$('#mg-status').textContent = cleanErr(e);
	}
}

aw.on('migrate:progress', ({ text, fraction }) => {
	$('#mg-status').textContent = text;
	if (typeof fraction === 'number') { $('#mg-progress').classList.remove('hidden'); $('#mg-bar').style.width = Math.round(fraction * 100) + '%'; }
});

$('#mg-check').addEventListener('click', async () => {
	const target = $('#mg-version').value;
	if (!target || !mgInst) return;
	$('#mg-check').disabled = true;
	$('#mg-results').innerHTML = '';
	$('#mg-apply').classList.add('hidden');
	try {
		mgPlan = await aw.migrateAnalyze(mgInst.id, target);
		renderMigrate();
	} catch (e) {
		$('#mg-status').textContent = cleanErr(e);
	} finally {
		$('#mg-check').disabled = false;
		$('#mg-progress').classList.add('hidden');
	}
});

function renderMigrate() {
	const p = mgPlan;
	const ok = p.items.filter((i) => i.status === 'ok').length;
	const miss = p.items.length - ok;
	if (!p.fabricOk) {
		$('#mg-status').textContent = `Fabric пока не поддерживает ${p.target} — моды перенести нельзя.`;
		return;
	}
	$('#mg-status').innerHTML = p.items.length
		? `<div class="mg-summary">Для <b>${esc(p.target)}</b>: найдено <b class="ok-text">${ok}</b> из ${p.items.length}${miss ? `, нет версии у <b style="color:#ff8f88">${miss}</b> — для них ниже похожие моды` : ' — всё переносится!'}</div>`
		: 'В сборке нет своих модов — перенесётся только версия игры.';
	const typeName = { mod: 'мод', resourcepack: 'ресурспак', shader: 'шейдер' };
	$('#mg-results').innerHTML = p.items.map((it, i) => `
		<div class="mg-row">
			<span class="${it.status === 'ok' ? 'ok' : 'miss'}">${it.status === 'ok' ? '✓' : '✗'}</span>
			<div><b>${esc(it.title)}</b> <span class="muted small">${typeName[it.type] || ''}</span>
				<div class="muted small">${it.status === 'ok' ? (it.from ? `${esc(it.from)} → ${esc(it.to)}` : `будет версия ${esc(it.to)}`) : `нет версии для ${esc(p.target)}`}</div></div>
			<span></span>
			${it.status === 'missing' ? `<div class="mg-alts">${(it.alternatives || []).length
				? '<div class="muted small">Можно заменить на:</div>' + it.alternatives.map((a) => `
					<label class="mg-alt"><input type="checkbox" data-alt="${i}" data-pid="${esc(a.projectId)}">
						<img src="${esc(a.icon || '')}" alt="" loading="lazy"><span><b>${esc(a.title)}</b> <span class="muted">— ${esc(a.description || '').slice(0, 70)}</span></span></label>`).join('')
				: '<div class="muted small">Похожих модов не нашлось — сборка перенесётся без него.</div>'}</div>` : ''}
		</div>`).join('');
	$('#mg-worlds-wrap').classList.remove('hidden');
	$('#mg-apply').textContent = `Создать сборку на ${p.target}`;
	$('#mg-apply').classList.remove('hidden');
}

$('#mg-apply').addEventListener('click', async () => {
	if (!mgPlan) return;
	const chosen = $$('[data-alt]:checked').map((c) => {
		const it = mgPlan.items[Number(c.dataset.alt)];
		return { projectId: c.dataset.pid, key: it.key, type: it.type };
	});
	$('#mg-apply').disabled = true;
	$('#mg-check').disabled = true;
	try {
		const r = await aw.migrateApply(mgInst.id, mgPlan.target, { items: mgPlan.items, chosen, copyWorlds: $('#mg-worlds').checked });
		$('#modal-migrate').classList.add('hidden');
		S.selected = r.instance.id;
		await reloadInstances();
		renderBuilds();
		if (r.failed.length) alert(`Сборка создана, но ${r.failed.length} мод(ов) не удалось поставить — их можно добавить вручную из каталога.`);
	} catch (e) {
		$('#mg-status').textContent = cleanErr(e);
	} finally {
		$('#mg-apply').disabled = false;
		$('#mg-check').disabled = false;
		$('#mg-progress').classList.add('hidden');
	}
});

// ================= Каталог =================
function catTarget() {
	return S.instances.find((i) => i.id === $('#catalog-target').value) || S.instances[0];
}

async function openCatalog() {
	await reloadInstances();
	await renderFilters();
	runSearch();
}

async function renderFilters() {
	const box = $('#filters');
	box.innerHTML = '<div class="muted small">Загрузка категорий...</div>';
	try {
		const cats = await aw.categories(S.cat.type);
		const groups = {};
		for (const c of cats) (groups[c.header] = groups[c.header] || []).push(c);
		const names = { categories: 'Категории', features: 'Особенности', performance_impact: 'Нагрузка', resolutions: 'Разрешение' };
		box.innerHTML = Object.entries(groups).map(([h, list]) => `<h3>${esc(names[h] || h)}</h3>` +
			list.map((c) => `<div class="cat ${S.cat.categories.has(c.name) ? 'on' : ''}" data-cat="${esc(c.name)}">${DOMPurify.sanitize(c.icon || '')}<span>${esc(c.name.replace(/-/g, ' '))}</span></div>`).join('')).join('');
		$$('.cat').forEach((el) => el.addEventListener('click', () => {
			const n = el.dataset.cat;
			S.cat.categories.has(n) ? S.cat.categories.delete(n) : S.cat.categories.add(n);
			el.classList.toggle('on');
			S.cat.offset = 0;
			runSearch();
		}));
	} catch {
		box.innerHTML = '<div class="muted small">Категории недоступны</div>';
	}
}

$$('#type-tabs .tab').forEach((t) => t.addEventListener('click', () => {
	$$('#type-tabs .tab').forEach((x) => x.classList.toggle('active', x === t));
	S.cat.type = t.dataset.type;
	S.cat.categories.clear();
	S.cat.offset = 0;
	renderFilters();
	runSearch();
}));
let searchTimer;
$('#catalog-query').addEventListener('input', (e) => {
	clearTimeout(searchTimer);
	searchTimer = setTimeout(() => { S.cat.query = e.target.value; S.cat.offset = 0; runSearch(); }, 350);
});
$('#catalog-sort').addEventListener('change', (e) => { S.cat.sort = e.target.value; S.cat.offset = 0; runSearch(); });
$('#catalog-target').addEventListener('change', () => { S.cat.target = $('#catalog-target').value; S.cat.offset = 0; runSearch(); });

let searchSeq = 0;
async function runSearch() {
	const seq = ++searchSeq;
	const inst = catTarget();
	const box = $('#catalog-results');
	if (S.cat.type === 'mod' && inst.loader === 'vanilla') {
		box.innerHTML = '<div class="card muted">В сборку «без модов» нельзя ставить моды. Выбери другую сборку или вкладку «Ресурспаки».</div>';
		$('#catalog-info').textContent = '';
		$('#pager').innerHTML = '';
		return;
	}
	box.innerHTML = '<div class="muted small">Ищу...</div>';
	try {
		const res = await aw.search({
			type: S.cat.type, query: S.cat.query, sort: S.cat.sort, offset: S.cat.offset,
			categories: [...S.cat.categories], mcVersion: inst.mcVersion,
		});
		if (seq !== searchSeq) return;
		const key = { mod: 'mods', resourcepack: 'resourcepacks', shader: 'shaders' }[S.cat.type];
		const have = new Set((inst[key] || []).map((x) => x.projectId));
		$('#catalog-info').textContent = `Найдено: ${fmtNum(res.total)} · для Minecraft ${inst.mcVersion}`;
		box.innerHTML = res.hits.map((h) => `
			<div class="mr-card" data-open="${esc(h.projectId)}">
				<img class="icon" src="${esc(h.icon || '')}" alt="" loading="lazy" decoding="async">
				<div>
					<div class="mr-title">${esc(h.title)}<span class="mr-author">от ${esc(h.author)}</span></div>
					<div class="mr-desc">${esc(h.description)}</div>
					<div class="chips">${h.categories.slice(0, 5).map((c) => `<span class="chip">${esc(String(c).replace(/-/g, ' '))}</span>`).join('')}</div>
				</div>
				<div class="mr-stats">
					<span><b>${fmtNum(h.downloads)}</b> скачиваний</span>
					<span><b>${fmtNum(h.follows)}</b> подписчиков</span>
					<span>обновлён ${fmtDate(h.updated)}</span>
					<button class="btn-small ${have.has(h.projectId) ? '' : 'btn-accent'}" data-install="${esc(h.projectId)}" ${have.has(h.projectId) ? 'disabled' : ''}>${have.has(h.projectId) ? 'Установлен' : 'Установить'}</button>
				</div>
			</div>`).join('') || '<div class="card muted">Ничего не нашлось для этой версии.</div>';
		$$('[data-open]').forEach((el) => el.addEventListener('click', (e) => { if (!e.target.closest('button')) openProject(el.dataset.open); }));
		$$('[data-install]').forEach((b) => b.addEventListener('click', () => installFromCatalog(b, b.dataset.install)));
		renderPager(res.total);
	} catch (e) {
		if (seq === searchSeq) box.innerHTML = `<div class="error">${esc(cleanErr(e))}</div>`;
	}
}

function renderPager(total) {
	const page = Math.floor(S.cat.offset / 20) + 1;
	const pages = Math.max(1, Math.ceil(total / 20));
	$('#pager').innerHTML = `<button class="btn-small" id="pg-prev" ${page <= 1 ? 'disabled' : ''}>← Назад</button>
		<span class="muted small" style="align-self:center">Страница ${page} из ${pages}</span>
		<button class="btn-small" id="pg-next" ${page >= pages ? 'disabled' : ''}>Вперёд →</button>`;
	$('#pg-prev').onclick = () => { S.cat.offset -= 20; runSearch(); $('main').scrollTop = 0; };
	$('#pg-next').onclick = () => { S.cat.offset += 20; runSearch(); $('main').scrollTop = 0; };
}

async function installFromCatalog(btn, projectId, versionId) {
	const inst = catTarget();
	btn.disabled = true;
	const old = btn.textContent;
	btn.textContent = 'Ставлю...';
	try {
		await aw.install(inst.id, projectId, S.cat.type, versionId);
		await reloadInstances();
		btn.textContent = 'Установлен';
	} catch (e) {
		btn.disabled = false;
		btn.textContent = old;
		alert(cleanErr(e));
	}
}

// ---- Карточка проекта ----
async function openProject(id) {
	const modal = $('#modal-project');
	const box = $('#project-box');
	modal.classList.remove('hidden');
	box.innerHTML = '<div class="pj-body muted">Загрузка...</div>';
	try {
		const p = await aw.project(id);
		const inst = catTarget();
		const links = Object.entries({ 'Страница Modrinth': p.links.page, 'Исходный код': p.links.source, 'Баг-трекер': p.links.issues, 'Вики': p.links.wiki, Discord: p.links.discord })
			.filter(([, u]) => u).map(([n, u]) => `<button class="btn-small" data-ext="${esc(u)}">${esc(n)}</button>`).join('');
		box.innerHTML = `
			<div class="pj-head">
				<img src="${esc(p.icon || '')}" alt="">
				<div>
					<h2>${esc(p.title)}</h2>
					<div class="muted">${esc(p.description)}</div>
					<div class="chips" style="margin-top:8px">${p.categories.map((c) => `<span class="chip">${esc(c.replace(/-/g, ' '))}</span>`).join('')}</div>
					<div class="pj-links">${links}</div>
				</div>
				<div class="mr-stats">
					<span><b>${fmtNum(p.downloads)}</b> скачиваний</span>
					<span><b>${fmtNum(p.follows)}</b> подписчиков</span>
					<button class="btn-small btn-accent" id="pj-install">Установить в «${esc(inst.name)}»</button>
					<button class="btn-small" data-close-project>Закрыть</button>
				</div>
			</div>
			<div class="pj-tabs">
				<button class="pj-tab active" data-tab="desc">Описание</button>
				<button class="pj-tab" data-tab="gallery">Галерея (${p.gallery.length})</button>
				<button class="pj-tab" data-tab="versions">Версии</button>
			</div>
			<div class="pj-body" id="pj-body"></div>`;
		const showTab = async (tab) => {
			$$('.pj-tab').forEach((t) => t.classList.toggle('active', t.dataset.tab === tab));
			const body = $('#pj-body');
			if (tab === 'desc') {
				body.innerHTML = `<div class="markdown">${DOMPurify.sanitize(marked.parse(p.body || ''), { FORBID_TAGS: ['iframe', 'style'] })}</div>`;
			} else if (tab === 'gallery') {
				body.innerHTML = p.gallery.length
					? `<div class="gallery">${p.gallery.map((g) => `<figure><img src="${esc(g.url)}" alt="" loading="lazy" decoding="async"><figcaption>${esc(g.title || '')}</figcaption></figure>`).join('')}</div>`
					: '<div class="muted">Скриншотов нет</div>';
			} else {
				body.innerHTML = '<div class="muted">Загрузка версий...</div>';
				try {
					const vs = await aw.projectVersions(p.id, inst.mcVersion, S.cat.type);
					const typeName = { release: 'Релиз', beta: 'Бета', alpha: 'Альфа' };
					body.innerHTML = vs.length ? vs.map((v) => `<div class="ver-row">
						<div><b>${esc(v.name || v.versionNumber)}</b><div class="muted small">${esc(v.file)}</div></div>
						<span><span class="vt ${esc(v.type)}">${esc(typeName[v.type] || v.type)}</span></span>
						<span class="muted small">${fmtDate(v.date)}</span>
						<button class="btn-small" data-ver="${esc(v.versionId)}">Установить</button></div>`).join('')
						: `<div class="muted">Нет версий для Minecraft ${esc(inst.mcVersion)}</div>`;
					$$('[data-ver]').forEach((b) => b.addEventListener('click', () => installFromCatalog(b, p.id, b.dataset.ver)));
				} catch (e) {
					body.innerHTML = `<div class="error">${esc(cleanErr(e))}</div>`;
				}
			}
			// ссылки из описания — во внешний браузер
			$$('#pj-body a').forEach((a) => a.addEventListener('click', (e) => { e.preventDefault(); if (a.href) aw.openExternal(a.href); }));
		};
		$$('.pj-tab').forEach((t) => t.addEventListener('click', () => showTab(t.dataset.tab)));
		$$('[data-ext]').forEach((b) => b.addEventListener('click', () => aw.openExternal(b.dataset.ext)));
		$('[data-close-project]').onclick = () => modal.classList.add('hidden');
		$('#pj-install').onclick = (e) => installFromCatalog(e.target, p.id).then(() => runSearch());
		showTab('desc');
	} catch (e) {
		box.innerHTML = `<div class="pj-body"><div class="error">${esc(cleanErr(e))}</div><div class="row-end"><button class="btn-small" data-close-project>Закрыть</button></div></div>`;
		$('[data-close-project]').onclick = () => modal.classList.add('hidden');
	}
}

// ================= Java =================
const JAVA_USE = { 8: 'Minecraft 1.16.5 и более ранние', 11: 'старые моды и сервера', 17: 'Minecraft 1.17 – 1.20.4', 21: 'Minecraft 1.20.5 – 1.21.x', 25: 'Minecraft 26.x (сервер AuraWorld)' };
const javaBusy = new Set();

async function renderJava() {
	const list = await aw.javaList();
	$('#java-list').innerHTML = list.map((j) => `
		<div class="java-row" id="java-${j.major}">
			<div class="java-major">Java ${j.major}</div>
			<div>
				<div>${esc(JAVA_USE[j.major] || '')}</div>
				<div class="muted small" data-jtext>${j.installed ? 'Установлена' : 'Не установлена'}</div>
				<div class="progress ${javaBusy.has(j.major) ? '' : 'hidden'}"><div class="progress-bar" data-jbar></div></div>
			</div>
			<div>${j.installed
				? `<button class="btn-small btn-danger" data-jrm="${j.major}">Удалить</button>`
				: `<button class="btn-small btn-accent" data-jin="${j.major}" ${javaBusy.has(j.major) ? 'disabled' : ''}>Скачать</button>`}</div>
		</div>`).join('');
	$$('[data-jin]').forEach((b) => b.addEventListener('click', async () => {
		const major = Number(b.dataset.jin);
		javaBusy.add(major);
		b.disabled = true;
		$(`#java-${major} .progress`).classList.remove('hidden');
		try { await aw.javaInstall(major); } catch (e) { alert(cleanErr(e)); }
		javaBusy.delete(major);
		renderJava();
	}));
	$$('[data-jrm]').forEach((b) => b.addEventListener('click', async () => {
		if (!confirm(`Удалить Java ${b.dataset.jrm}? При необходимости она скачается снова.`)) return;
		await aw.javaRemove(Number(b.dataset.jrm));
		renderJava();
	}));
}
aw.on('java:progress', ({ major, text, fraction }) => {
	const row = $(`#java-${major}`);
	if (!row) return;
	row.querySelector('[data-jtext]').textContent = text;
	if (typeof fraction === 'number') row.querySelector('[data-jbar]').style.width = Math.round(fraction * 100) + '%';
});

// ================= Аккаунты =================
function renderAccounts() {
	const list = S.settings.accounts || [];
	$('#account-list').innerHTML = list.length ? list.map((a) => `
		<div class="acc-row ${a.active ? 'active' : ''}">
			<img class="avatar" src="${head(a.name)}" alt="">
			<div class="grow"><b>${esc(a.name)}</b> ${a.type === 'msa' ? '<span class="badge ms">Microsoft</span>' : '<span class="badge lock">по нику</span>'}</div>
			${a.active ? '<span class="badge">выбран</span>' : `<button class="btn-small" data-acc-use="${esc(a.id)}">Выбрать</button>`}
			<button class="btn-small btn-danger" data-acc-rm="${esc(a.id)}">Удалить</button>
		</div>`).join('') : '<div class="muted small">Аккаунтов пока нет.</div>';
	fixAvatars();
	const ready = S.config.microsoftReady;
	$('#login-ms').disabled = !ready;
	$('#ms-hint').textContent = ready ? 'Откроется окно входа Microsoft. Пароль вводится только на сайте Microsoft — лаунчер его не видит.' : 'Вход через Microsoft включится, когда в лаунчер будет добавлен одобренный ID приложения.';
	$$('[data-acc-use]').forEach((b) => b.addEventListener('click', async () => setAccounts(await aw.selectAccount(b.dataset.accUse))));
	$$('[data-acc-rm]').forEach((b) => b.addEventListener('click', async () => setAccounts(await aw.removeAccount(b.dataset.accRm))));
}
function setAccounts(list) {
	S.settings.accounts = list;
	renderAccounts();
	renderAccountChip();
}
function accountError(e) {
	$('#account-error').textContent = e ? cleanErr(e) : '';
	$('#account-error').classList.toggle('hidden', !e);
}
$('#add-offline').addEventListener('click', async () => {
	accountError(null);
	try { setAccounts(await aw.addOffline($('#offline-nick').value.trim())); $('#offline-nick').value = ''; } catch (e) { accountError(e); }
});
$('#offline-nick').addEventListener('keydown', (e) => e.key === 'Enter' && $('#add-offline').click());
$('#login-ms').addEventListener('click', async () => {
	accountError(null);
	$('#login-ms').disabled = true;
	try { setAccounts(await aw.loginMicrosoft()); } catch (e) { accountError(e); }
	$('#login-ms').disabled = !S.config.microsoftReady;
});

// ================= Темы =================
const THEMES = [
	{ id: 'default', name: 'Стандартная' },
	{ id: 'cave', name: 'Пышная пещера', img: 'backgrounds/cave.webp', thumb: 'backgrounds/cave-thumb.webp' },
	{ id: 'depths', name: 'Глубины', img: 'backgrounds/depths.webp', thumb: 'backgrounds/depths-thumb.webp' },
	{ id: 'night', name: 'Звёздная ночь', img: 'backgrounds/night.webp', thumb: 'backgrounds/night-thumb.webp' },
	{ id: 'custom', name: 'Своя картинка' },
];

function themeImage(id) {
	if (id === 'custom') return S.settings.customBg;
	const t = THEMES.find((x) => x.id === id);
	return t && t.img;
}

function applyTheme() {
	const img = themeImage(S.settings.theme);
	const bg = $('#bg');
	if (img) {
		const pre = new Image();
		pre.onload = () => { bg.style.backgroundImage = `url("${img}")`; bg.classList.add('on'); };
		pre.src = img;
		document.body.classList.add('themed');
	} else {
		bg.classList.remove('on');
		document.body.classList.remove('themed');
	}
}

function themeCards(ids) {
	return THEMES.filter((t) => !ids || ids.includes(t.id)).map((t) => {
		let prev = '';
		if (t.id === 'custom' && S.settings.customBg) prev = `style="background-image:url('${esc(S.settings.customBg)}')"`;
		else if (t.thumb) prev = `style="background-image:url('${t.thumb}')"`;
		const inner = t.id === 'custom' && !S.settings.customBg ? '+' : '';
		return `<div class="theme-card ${S.settings.theme === t.id ? 'on' : ''}" data-theme="${t.id}">
			<div class="preview ${t.id === 'default' ? 'default' : ''}" ${prev}>${inner}</div>
			<div class="label">${esc(t.name)}</div></div>`;
	}).join('');
}

async function setTheme(id) {
	if (id === 'custom') {
		const r = await aw.pickCustomBg();
		if (!r) return;
		S.settings = r;
	} else {
		S.settings = await aw.saveSettings({ theme: id });
	}
	applyTheme();
	renderThemes();
}

function bindThemeCards(scope, after) {
	scope.querySelectorAll('[data-theme]').forEach((el) => el.addEventListener('click', async () => {
		await setTheme(el.dataset.theme);
		if (after) after();
	}));
}

function renderThemes() {
	$('#themes').innerHTML = themeCards();
	bindThemeCards($('#themes'));
}

// ================= Настройки =================
function renderSettings() {
	const s = S.settings;
	$('#root-path').textContent = s.root;
	$('#memory').value = Math.round(s.memoryMb / 1024);
	$('#mem-val').textContent = Math.round(s.memoryMb / 1024);
	$('#hide-launch').checked = s.hideOnLaunch;
	renderThemes();
}
$('#memory').addEventListener('input', (e) => ($('#mem-val').textContent = e.target.value));
$('#memory').addEventListener('change', async (e) => { S.settings = await aw.saveSettings({ memoryMb: Number(e.target.value) * 1024 }); });
$('#hide-launch').addEventListener('change', async (e) => { S.settings = await aw.saveSettings({ hideOnLaunch: e.target.checked }); });
async function moveRootTo(dir) {
	S.settings = await aw.moveRoot(dir);
	renderSettings();
	await reloadInstances();
}
$('#choose-root').addEventListener('click', async () => {
	const dir = await aw.chooseFolder();
	if (!dir) return;
	if (!confirm(`Перенести папку игры в:\n${dir}\n\nВсё уже скачанное (игра, сборки, миры) переедет туда.`)) return;
	try { await moveRootTo(dir); } catch (e) { alert(cleanErr(e)); }
});
$('#show-guide').addEventListener('click', () => openGuide());

// ================= Гайд первого запуска =================
let guideStep = 0;
const errText = (t) => `<span style="color:#ffb3ae">${esc(t)}</span>`;
const GUIDE = [
	() => `<div class="big-icon">◆</div><h2>Добро пожаловать в ${esc(S.config.serverName)}!</h2>
		<p class="muted">Это лаунчер нашего сервера. За минуту настроим всё нужное: папку игры, ник и внешний вид.</p>
		<p class="muted small">Все настройки потом можно поменять во вкладке «Настройки».</p>`,
	() => `<h2>Где будет игра</h2>
		<p class="muted">Сюда лаунчер скачает Minecraft, Java, моды и твои миры. Нужно примерно 1–3 ГБ свободного места.</p>
		<div class="guide-path" id="g-path">${esc(S.settings.root)}</div>
		<div style="display:flex;gap:8px">
			<button class="btn-small btn-accent" id="g-keep">Оставить здесь</button>
			<button class="btn-small" id="g-move">Выбрать другую папку...</button>
		</div>
		<div class="small" id="g-path-status"></div>`,
	() => {
		const a = activeAccount();
		return `<h2>Твой ник</h2>
		<p class="muted">Под этим ником ты будешь на сервере. Латиница, цифры и _, от 3 до 16 символов.</p>
		<div class="search-row"><input id="g-nick" maxlength="16" placeholder="Например, Steve_2026" value="${esc(a ? a.name : '')}" spellcheck="false"><button class="btn-small btn-accent" id="g-nick-save">Сохранить</button></div>
		<div class="small" id="g-nick-status">${a ? `<span class="ok-text">✓ Аккаунт «${esc(a.name)}» выбран</span>` : ''}</div>
		<p class="muted small">Сменить ник или добавить ещё аккаунты можно во вкладке <b>«Аккаунты»</b>.</p>`;
	},
	() => `<h2>Сборки</h2>
		<p class="muted">Сборка — это набор модов со своими мирами и настройками.</p>
		<ol class="guide-steps">
			<li><b>Официальная сборка</b> уже готова: в ней всё нужное для сервера. Просто нажми «Играть».</li>
			<li>Хочешь свою? Открой <b>«Сборки» → «+ Новая»</b>, выбери версию игры и Fabric.</li>
			<li>Добавляй моды, ресурспаки и шейдеры во вкладке <b>«Каталог»</b> — это Modrinth прямо в лаунчере.</li>
			<li>Моды сервера подкладываются в сборки на версии сервера сами — их не нужно искать.</li>
		</ol>`,
	() => `<h2>Выбери оформление</h2>
		<p class="muted">Потом можно поменять или поставить свою картинку в «Настройках».</p>
		<div class="guide-themes" id="g-themes">${themeCards(['default', 'night'])}</div>`,
	() => `<div class="big-icon">✓</div><h2>Всё готово!</h2>
		<p class="muted">Жми <b>«Играть»</b>. При первом запуске лаунчер скачает игру и Java — это займёт несколько минут, дальше запуск будет быстрым.</p>`,
];

function openGuide() {
	guideStep = 0;
	$('#modal-guide').classList.remove('hidden');
	renderGuide();
}

function renderGuide() {
	$('#guide-dots').innerHTML = GUIDE.map((_, i) => `<i class="${i === guideStep ? 'on' : ''}"></i>`).join('');
	const step = $('#guide-step');
	step.innerHTML = GUIDE[guideStep]();
	step.style.animation = 'none';
	void step.offsetWidth;
	step.style.animation = '';
	$('#guide-back').style.visibility = guideStep === 0 ? 'hidden' : 'visible';
	$('#guide-next').textContent = guideStep === GUIDE.length - 1 ? 'Начать' : 'Далее';

	if ($('#g-keep')) {
		$('#g-keep').onclick = () => { guideStep++; renderGuide(); };
		$('#g-move').onclick = async () => {
			const dir = await aw.chooseFolder();
			if (!dir) return;
			$('#g-path-status').textContent = 'Переношу...';
			try {
				await moveRootTo(dir);
				$('#g-path').textContent = S.settings.root;
				$('#g-path-status').innerHTML = '<span class="ok-text">✓ Папка выбрана</span>';
			} catch (e) {
				$('#g-path-status').innerHTML = errText(cleanErr(e));
			}
		};
	}
	if ($('#g-nick-save')) {
		const save = async () => {
			try {
				setAccounts(await aw.addOffline($('#g-nick').value.trim()));
				$('#g-nick-status').innerHTML = `<span class="ok-text">✓ Аккаунт «${esc(activeAccount().name)}» выбран</span>`;
			} catch (e) {
				$('#g-nick-status').innerHTML = errText(cleanErr(e));
			}
		};
		$('#g-nick-save').onclick = save;
		$('#g-nick').addEventListener('keydown', (e) => e.key === 'Enter' && save());
	}
	if ($('#g-themes')) bindThemeCards($('#g-themes'), () => renderGuide());
}

$('#guide-back').addEventListener('click', () => { if (guideStep > 0) { guideStep--; renderGuide(); } });
$('#guide-next').addEventListener('click', async () => {
	// Без аккаунта играть нельзя — на шаге с ником он обязателен
	if ($('#g-nick') && !activeAccount()) {
		const nick = $('#g-nick').value.trim();
		if (!nick) { $('#g-nick-status').innerHTML = errText('Введи ник, чтобы продолжить'); return; }
		try { setAccounts(await aw.addOffline(nick)); } catch (e) { $('#g-nick-status').innerHTML = errText(cleanErr(e)); return; }
	}
	if (guideStep < GUIDE.length - 1) { guideStep++; renderGuide(); return; }
	$('#modal-guide').classList.add('hidden');
	S.settings = await aw.saveSettings({ onboarded: true });
	renderSettings();
	renderAccountChip();
});

init().then(() => {
	applyTheme();
	if (!S.settings.onboarded) openGuide();
});
