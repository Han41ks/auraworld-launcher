'use strict';
// Мост между интерфейсом и ядром на Rust (вместо preload.js из Electron).
(() => {
	const T = window.__TAURI__;

	/** Путь к своей картинке фона → адрес, который может показать окно. */
	function bgUrl(v) {
		if (!v || /^(asset:|https?:\/\/asset\.localhost)/.test(v)) return v;
		let p = v;
		if (p.startsWith('file:')) {
			// настройки от прежней версии лаунчера
			try { p = decodeURIComponent(new URL(p).pathname); } catch { return null; }
			if (/^\/[A-Za-z]:\//.test(p)) p = p.slice(1).replace(/\//g, '\\');
		}
		return T.core.convertFileSrc(p);
	}
	const fix = (r) => {
		if (r && typeof r === 'object' && !Array.isArray(r) && r.customBg) r.customBg = bgUrl(r.customBg);
		return r;
	};

	const call = (channel) => async (...args) => {
		try {
			return fix(await T.core.invoke('call', { channel, args: args.map((a) => (a === undefined ? null : a)) }));
		} catch (e) {
			throw new Error(typeof e === 'string' ? e : (e && e.message) || String(e));
		}
	};

	window.aw = {
		config: call('config:get'),
		settings: call('settings:get'),
		saveSettings: call('settings:set'),
		chooseFolder: call('dialog:folder'),
		openExternal: call('open:external'),
		pickCustomBg: call('theme:pickCustom'),
		moveRoot: call('settings:moveRoot'),

		addOffline: call('accounts:addOffline'),
		loginMicrosoft: call('accounts:loginMicrosoft'),
		selectAccount: call('accounts:select'),
		removeAccount: call('accounts:remove'),

		instances: call('instances:list'),
		createInstance: call('instances:create'),
		removeInstance: call('instances:remove'),
		openInstance: call('instances:open'),
		removeContent: call('instances:removeContent'),
		versions: call('versions:list'),

		search: call('catalog:search'),
		categories: call('catalog:categories'),
		project: call('catalog:project'),
		projectVersions: call('catalog:versions'),
		install: call('catalog:install'),
		manifest: call('manifest:get'),

		javaList: call('java:list'),
		javaInstall: call('java:install'),
		javaRemove: call('java:remove'),

		skinsList: call('skins:list'),
		skinsSave: call('skins:save'),
		skinsRemove: call('skins:remove'),
		skinsSetActive: call('skins:setActive'),
		skinsFromLicensed: call('skins:fromLicensed'),
		skinsTemplates: call('skins:templates'),
		skinsPickFile: call('skins:pickFile'),
		skinsExport: call('skins:export'),

		migrateAnalyze: call('migrate:analyze'),
		migrateApply: call('migrate:apply'),

		play: call('game:play'),
		on: (channel, fn) => { T.event.listen(channel, (e) => fn(e.payload)); },
	};

	// Обычное контекстное меню браузера в лаунчере ни к чему (кроме полей ввода)
	window.addEventListener('contextmenu', (e) => {
		if (!e.target.closest('input, textarea')) e.preventDefault();
	});
})();
