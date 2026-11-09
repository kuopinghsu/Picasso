(function () {
	'use strict';
	var text = window.vlText || {};
	var html = document.documentElement;
	// The Customizer's colour mode (system, light or dark), and whether visitors may change it with the header's button.
	var mode = text.colorMode || 'system';
	var canSwitch = text.colorToggle !== '';
	var stored;
	if (canSwitch) {
		try { stored = localStorage.getItem('vl-theme'); } catch (e) {}
	}
	var mq = window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)');
	var theme = (stored === 'dark' || stored === 'light') ? stored : (mode !== 'system' ? mode : (mq && mq.matches ? 'dark' : 'light'));
	function apply(value) {
		html.setAttribute('data-theme', value);
		// The button keeps its name, "Dark", and is pressed while the page is dark.
		document.querySelectorAll('.theme-toggle').forEach(function (button) {
			button.setAttribute('aria-pressed', value === 'dark' ? 'true' : 'false');
		});
	}
	// Sub-menus opened with their button (touch screens and keyboards); a mouse also opens them by hovering.
	function closeSubs(except) {
		document.querySelectorAll('.nav-list .is-open').forEach(function (item) {
			if (item === except) return;
			item.classList.remove('is-open');
			var button = item.querySelector(':scope > .sub-toggle');
			if (button) button.setAttribute('aria-expanded', 'false');
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
		var sub = e.target.closest('.sub-toggle');
		if (sub) {
			var item = sub.parentNode;
			var opened = !item.classList.contains('is-open');
			closeSubs(item);
			item.classList.toggle('is-open', opened);
			sub.setAttribute('aria-expanded', String(opened));
		} else if (!e.target.closest('.nav-list .is-open')) {
			closeSubs();
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
		var openSub = document.querySelector('.nav-list .is-open');
		if (openSub) {
			closeSubs();
			var subButton = openSub.querySelector(':scope > .sub-toggle');
			if (subButton) subButton.focus();
			return;
		}
		var nav = document.getElementById('primary-nav');
		var menu = document.querySelector('.menu-toggle');
		if (nav && menu && nav.classList.contains('is-open')) {
			nav.classList.remove('is-open');
			menu.setAttribute('aria-expanded', 'false');
			menu.focus();
		}
	});
	// Tabbing out of an open sub-menu closes it.
	document.addEventListener('focusout', function (e) {
		var item = e.target.closest && e.target.closest('.nav-list .is-open');
		if (item && !item.contains(e.relatedTarget)) closeSubs();
	});
	if (mq && mode === 'system') {
		mq.addEventListener('change', function (e) {
			try { if (canSwitch && localStorage.getItem('vl-theme')) return; } catch (ex) {}
			apply(e.matches ? 'dark' : 'light');
		});
	}

	// Tables in a post. A table pasted outside a Table block gets the same scrolling frame as one,
	// so it keeps full width and readable columns; a first row of header cells is marked as column
	// headers for screen readers; and a frame that scrolls can be reached and scrolled with the keyboard,
	// named by its caption, while one that fits adds no extra Tab stop.
	function tables() {
		var frames = [];
		document.querySelectorAll('.article-content > table').forEach(function (table) {
			var frame = document.createElement('div');
			frame.className = 'vl-table';
			table.parentNode.insertBefore(frame, table);
			frame.appendChild(table);
		});
		document.querySelectorAll('.article-content table').forEach(function (table) {
			var first = table.tHead ? null : table.rows[0];
			if (first && first.cells.length && Array.prototype.every.call(first.cells, function (cell) { return cell.tagName === 'TH'; })) {
				Array.prototype.forEach.call(first.cells, function (cell) {
					if (!cell.hasAttribute('scope')) cell.setAttribute('scope', 'col');
				});
			}
			var frame = table.closest('.wp-block-table, .vl-table');
			if (frame && frames.indexOf(frame) < 0) frames.push(frame);
		});
		frames.forEach(function (frame, i) {
			var caption = frame.querySelector('figcaption');
			if (caption && !caption.id) caption.id = 'vl-table-caption-' + (i + 1);
			function update() {
				if (frame.scrollWidth > frame.clientWidth + 1) {
					frame.setAttribute('tabindex', '0');
					if (caption) {
						frame.setAttribute('role', 'region');
						frame.setAttribute('aria-labelledby', caption.id);
					}
				} else {
					frame.removeAttribute('tabindex');
					frame.removeAttribute('role');
					frame.removeAttribute('aria-labelledby');
				}
			}
			update();
			if (window.ResizeObserver) new ResizeObserver(update).observe(frame);
		});
	}
	if (document.readyState === 'loading') {
		document.addEventListener('DOMContentLoaded', tables);
	} else {
		tables();
	}
})();
