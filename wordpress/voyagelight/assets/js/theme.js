(function () {
	'use strict';
	var text = window.vlText || { dark: 'Dark', light: 'Light' };
	var html = document.documentElement;
	var stored;
	try { stored = localStorage.getItem('vl-theme'); } catch (e) {}
	var mq = window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)');
	var theme = (stored === 'dark' || stored === 'light') ? stored : (mq && mq.matches ? 'dark' : 'light');
	function apply(value) {
		html.setAttribute('data-theme', value);
		document.querySelectorAll('.theme-toggle').forEach(function (button) {
			button.setAttribute('aria-pressed', value === 'dark' ? 'true' : 'false');
			button.querySelector('.mode-icon').textContent = value === 'dark' ? '☀' : '☾';
			button.querySelector('.mode-label').textContent = value === 'dark' ? text.light : text.dark;
		});
	}
	apply(theme);
	document.addEventListener('click', function (e) {
		var toggle = e.target.closest('.theme-toggle');
		if (toggle) {
			theme = html.getAttribute('data-theme') === 'dark' ? 'light' : 'dark';
			apply(theme);
			try { localStorage.setItem('vl-theme', theme); } catch (ex) {}
		}
		var menu = e.target.closest('.menu-toggle');
		if (menu) {
			var nav = document.getElementById('primary-nav');
			if (nav) {
				var open = nav.classList.toggle('is-open');
				menu.setAttribute('aria-expanded', String(open));
			}
		}
	});
	document.addEventListener('keydown', function (e) {
		if (e.key !== 'Escape') return;
		var nav = document.getElementById('primary-nav');
		var menu = document.querySelector('.menu-toggle');
		if (nav && menu && nav.classList.contains('is-open')) {
			nav.classList.remove('is-open');
			menu.setAttribute('aria-expanded', 'false');
			menu.focus();
		}
	});
	if (mq) {
		mq.addEventListener('change', function (e) {
			try { if (localStorage.getItem('vl-theme')) return; } catch (ex) {}
			apply(e.matches ? 'dark' : 'light');
		});
	}
})();
