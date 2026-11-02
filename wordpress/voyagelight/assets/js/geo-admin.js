(function () {
	'use strict';
	const geo = window.vlGeo;
	const data = window.vlGeoAdmin || {};
	const text = data.text || {};
	const reasons = text.reasons || {};
	if (!geo) return;

	function fill(format, ...values) {
		let next = 0;
		return String(format || '').replace(/%(?:(\d)\$)?[sd]/g, (m, n) => String(values[n ? n - 1 : next++]));
	}
	function el(tag, className, content) {
		const node = document.createElement(tag);
		if (className) node.className = className;
		if (content !== undefined) node.textContent = content;
		return node;
	}
	function button(label, className, title) {
		const b = el('button', className || 'button', label);
		b.type = 'button';
		if (title) {
			b.title = title;
			b.setAttribute('aria-label', title);
		}
		return b;
	}
	const splitLines = (value) => value.split(/\r\n|\r|\n/);
	const shown = (point) => point[0].toFixed(5) + ', ' + point[1].toFixed(5);
	const changed = (field) => field.dispatchEvent(new Event('input', { bubbles: true }));
	const dayMax = +data.dayMax >= 1 ? +data.dayMax : geo.dayMax;
	const isDay = (value) => /^\d+$/.test(String(value)) && +value >= 1 && +value <= dayMax;

	let loader = null;
	function leaflet() {
		if (window.L) return Promise.resolve();
		if (loader) return loader;
		function add(tag, properties) {
			return new Promise((resolve, reject) => {
				const e = document.createElement(tag);
				Object.assign(e, properties);
				e.onload = resolve;
				e.onerror = reject;
				document.head.appendChild(e);
			});
		}
		loader = Promise.all([
			add('link', { rel: 'stylesheet', href: 'https://unpkg.com/leaflet@1.9.4/dist/leaflet.css', integrity: 'sha256-p4NxAoJBhIIN+hmNHrzRCf9tD/miZyoHS5obTRR9BMY=', crossOrigin: 'anonymous' }),
			add('script', { src: 'https://unpkg.com/leaflet@1.9.4/dist/leaflet.js', integrity: 'sha256-20nQCchB9co0qIjJZRGuk2/Z9VM+kNiyxNV1lvTlZBo=', crossOrigin: 'anonymous' })
		]).catch((error) => {
			loader = null;
			throw error;
		});
		return loader;
	}

	function lineProblems(area) {
		const kind = area.dataset.kind;
		const out = [];
		splitLines(area.value).slice(0, +area.dataset.maxLines).forEach((line, i) => {
			const row = kind === 'day' ? geo.dayLine(line) : geo.stopLine(line);
			if (typeof row === 'string') out.push(fill(text.problemLine, i + 1, line.trim(), reasons[row] || row));
		});
		return out;
	}
	function showProblems(area) {
		const box = document.getElementById(area.dataset.problems);
		if (!box) return;
		const list = lineProblems(area);
		const over = splitLines(area.value.replace(/\s+$/, '')).length > +area.dataset.maxLines || area.value.length > +area.dataset.maxChars;
		box.replaceChildren();
		if (!list.length && !over) return;
		const notice = el('div', 'notice notice-warning inline');
		if (over) notice.appendChild(el('p', '', fill(text.limits, area.dataset.maxLines, area.dataset.maxChars)));
		if (list.length) {
			notice.appendChild(el('p', '', text.problems));
			const ul = el('ul');
			list.forEach((line) => ul.appendChild(el('li', '', line)));
			notice.appendChild(ul);
		}
		box.appendChild(notice);
	}

	function locationReading(input) {
		const out = document.getElementById('vl-location-reading');
		const value = input.value.trim();
		const point = value === '' ? null : geo.read(value);
		if (!out) return point;
		out.classList.toggle('is-error', typeof point === 'string');
		if (value === '') out.textContent = text.noLocation;
		else out.textContent = Array.isArray(point) ? fill(text.readsAs, shown(point)) : (reasons[point] || '');
		return point;
	}

	/* The Pick on map panel: a map, a place search and a form naming a new stop. */
	function picker(root, hooks) {
		const open = root.querySelector('.vl-geo-open');
		const panel = root.querySelector('.vl-geo-panel');
		const host = root.querySelector('.vl-geo-map');
		const mapStatus = root.querySelector('.vl-geo-map-status');
		const form = root.querySelector('.vl-geo-add');
		const nameField = form.querySelector('.vl-geo-add-name');
		const dayField = form.querySelector('.vl-geo-add-day');
		const query = root.querySelector('.vl-geo-query');
		const find = root.querySelector('.vl-geo-find');
		const searchStatus = root.querySelector('.vl-geo-search-status');
		const results = root.querySelector('.vl-geo-results');
		let map = null;
		let pins = null;
		let pending = null;
		let draft = null;
		let lastSearch = 0;
		let timer = 0;

		function draw(fit) {
			if (!map) return;
			pins.clearLayers();
			const bounds = [];
			hooks.points().forEach((p) => {
				const marker = L.marker([p.lat, p.lon], { draggable: true, autoPan: true }).addTo(pins);
				marker.bindTooltip(p.name || text.place);
				marker.on('dragend', () => {
					const at = marker.getLatLng().wrap();
					p.move(at.lat, at.lng);
				});
				bounds.push([p.lat, p.lon]);
			});
			if (!fit) return;
			if (bounds.length > 1) map.fitBounds(bounds, { padding: [30, 30], maxZoom: 12 });
			else if (bounds.length) map.setView(bounds[0], 11);
			else map.setView([20, 0], 2);
		}
		function redraw() {
			clearTimeout(timer);
			timer = setTimeout(() => draw(false), 250);
		}
		function centre(lat, lon) {
			if (map) map.setView([lat, lon], Math.max(map.getZoom(), 12));
		}

		function closeForm() {
			form.hidden = true;
			pending = null;
			if (draft) draft.remove();
			draft = null;
		}
		function ask(lat, lon) {
			pending = [lat, lon];
			form.hidden = false;
			nameField.value = '';
			if (dayField) dayField.value = hooks.lastDay();
			if (draft) draft.remove();
			draft = map ? L.circleMarker([lat, lon], { radius: 9, color: '#b32d2e' }).addTo(map) : null;
			nameField.focus();
		}
		function confirmStop() {
			const name = nameField.value.trim();
			if (!name || !pending) {
				nameField.focus();
				return;
			}
			hooks.add(name, pending[0], pending[1], dayField ? dayField.value : '');
			closeForm();
		}
		form.querySelector('.vl-geo-add-ok').addEventListener('click', confirmStop);
		form.querySelector('.vl-geo-add-cancel').addEventListener('click', closeForm);
		form.addEventListener('keydown', (e) => {
			if (e.key === 'Enter') {
				e.preventDefault();
				confirmStop();
			} else if (e.key === 'Escape') {
				e.preventDefault();
				closeForm();
			}
		});

		open.addEventListener('click', () => {
			const show = panel.hidden;
			panel.hidden = !show;
			open.setAttribute('aria-expanded', String(show));
			open.textContent = show ? text.hideMap : text.pickOnMap;
			if (!show) return;
			if (map) {
				map.invalidateSize();
				return;
			}
			mapStatus.textContent = text.loadingMap;
			leaflet().then(() => {
				if (map) return;
				mapStatus.textContent = '';
				map = L.map(host, { scrollWheelZoom: false });
				L.tileLayer('https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png', { maxZoom: 18, attribution: '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors' }).addTo(map);
				pins = L.layerGroup().addTo(map);
				map.on('click', (e) => {
					const at = e.latlng.wrap();
					hooks.click(at.lat, at.lng);
				});
				draw(true);
				setTimeout(() => map.invalidateSize(), 100);
			}).catch(() => {
				mapStatus.textContent = text.mapFailed;
			});
		});

		function search() {
			const q = query.value.trim();
			if (!q) {
				query.focus();
				return;
			}
			if (find.disabled || Date.now() - lastSearch < 1000) return;
			lastSearch = Date.now();
			find.disabled = true;
			results.replaceChildren();
			searchStatus.textContent = text.searching;
			const params = new URLSearchParams({ format: 'jsonv2', limit: '5', q: q, 'accept-language': data.language || 'en' });
			fetch('https://nominatim.openstreetmap.org/search?' + params.toString(), { credentials: 'omit', headers: { Accept: 'application/json' } })
				.then((response) => {
					if (!response.ok) throw new Error(String(response.status));
					return response.json();
				})
				.then((list) => {
					const places = (Array.isArray(list) ? list : []).filter((p) => p && geo.validCoord(Number(p.lat), 'lat') !== null && geo.validCoord(Number(p.lon), 'lon') !== null);
					searchStatus.textContent = places.length ? '' : text.noResults;
					places.forEach((p) => {
						const label = String(p.display_name || p.name || '');
						const choose = button(label, 'button-link');
						choose.addEventListener('click', () => {
							const lat = Number(Number(p.lat).toFixed(6));
							const lon = Number(Number(p.lon).toFixed(6));
							hooks.choose(String(p.name || label.split(',')[0]).trim() || label, lat, lon);
							centre(lat, lon);
						});
						const item = el('li');
						item.appendChild(choose);
						results.appendChild(item);
					});
				})
				.catch(() => {
					searchStatus.textContent = text.searchFailed;
				})
				.then(() => {
					setTimeout(() => { find.disabled = false; }, 1000);
				});
		}
		find.addEventListener('click', search);
		query.addEventListener('keydown', (e) => {
			if (e.key === 'Enter') {
				e.preventDefault();
				search();
			}
		});

		return { ask, redraw };
	}

	function postBox() {
		const input = document.getElementById('vl_location');
		const stops = document.getElementById('vl_stops');
		const root = document.querySelector('.vl-geo-picker[data-kind="post"]');
		if (!input || !stops) return;
		locationReading(input);
		if (!root) return;
		const mode = root.querySelector('.vl-geo-click');
		function setLocation(lat, lon) {
			input.value = geo.format(lat, lon);
			changed(input);
		}
		function setLine(index, line) {
			const lines = splitLines(stops.value);
			lines[index] = line;
			stops.value = lines.join('\n');
			changed(stops);
		}
		function addStop(name, lat, lon) {
			const before = stops.value.replace(/\s+$/, '');
			stops.value = (before ? before + '\n' : '') + geo.joinStop({ name: name, location: geo.format(lat, lon), note: '' });
			changed(stops);
		}
		const pick = picker(root, {
			points() {
				const points = [];
				const here = geo.read(input.value);
				if (Array.isArray(here)) points.push({ lat: here[0], lon: here[1], name: text.storyLocation, move: setLocation });
				splitLines(stops.value).slice(0, +stops.dataset.maxLines).forEach((line, i) => {
					const stop = geo.stopLine(line);
					if (!stop || typeof stop !== 'object') return;
					points.push({
						lat: stop.lat,
						lon: stop.lon,
						name: stop.name,
						move: (lat, lon) => setLine(i, geo.joinStop(Object.assign(geo.splitStop(line), { location: geo.format(lat, lon) })))
					});
				});
				return points;
			},
			click(lat, lon) {
				if (mode && mode.value === 'stop') pick.ask(lat, lon);
				else setLocation(lat, lon);
			},
			add: addStop,
			choose(name, lat, lon) {
				if (mode && mode.value === 'stop') addStop(name, lat, lon);
				else setLocation(lat, lon);
			},
			lastDay: () => ''
		});
		input.addEventListener('input', () => {
			locationReading(input);
			pick.redraw();
		});
		stops.addEventListener('input', pick.redraw);
	}

	function tripBox() {
		const area = document.getElementById('vl_trip_days');
		const host = document.querySelector('.vl-trip-rows');
		const root = document.querySelector('.vl-geo-picker[data-kind="trip"]');
		if (!area || !host) return;
		const stories = Array.isArray(data.stories) ? data.stories : [];
		const known = new Set(stories.map((s) => String(s.id)));
		const parse = () => splitLines(area.value).filter((line) => line.trim() !== '').map(geo.splitDay);
		let rows = parse();
		let lastUsed = '';
		let asText = false;
		let pick = null;

		const lastDay = () => {
			if (isDay(lastUsed)) return lastUsed;
			for (let i = rows.length - 1; i >= 0; i--) {
				if (isDay(rows[i].day)) return String(rows[i].day);
			}
			return '1';
		};
		function write() {
			area.value = rows.filter((r) => r.name || r.location || r.note || r.post).map(geo.joinDay).join('\n');
			showProblems(area);
			if (pick) pick.redraw();
		}

		const wrap = el('div', 'vl-trip-table-wrap');
		const table = el('table', 'widefat striped vl-trip-table');
		const head = el('tr');
		[text.day, text.place, text.location, text.note, text.story, text.order].forEach((label) => {
			const th = el('th', '', label);
			th.scope = 'col';
			head.appendChild(th);
		});
		const thead = el('thead');
		thead.appendChild(head);
		const body = el('tbody');
		table.append(thead, body);
		wrap.appendChild(table);
		const actions = el('div', 'vl-trip-actions');
		const addRow = button(text.addRow);
		const addLinked = button(text.addLinked);
		const toggle = button(text.editText, 'button-link');
		const status = el('span', 'vl-trip-status');
		status.setAttribute('aria-live', 'polite');
		actions.append(addRow, addLinked, toggle, status);
		host.append(wrap, actions);
		area.hidden = true;

		function field(type, value, label) {
			const input = el('input');
			input.type = type;
			input.value = value;
			input.setAttribute('aria-label', label);
			return input;
		}
		function option(value, label) {
			const o = el('option', '', label);
			o.value = value;
			return o;
		}
		function storySelect(value) {
			const select = el('select');
			select.setAttribute('aria-label', text.story);
			select.appendChild(option('', text.noStory));
			[[text.linkedStories, true], [text.otherStories, false]].forEach(([label, linked]) => {
				const group = stories.filter((s) => !!s.linked === linked);
				if (!group.length) return;
				const optgroup = el('optgroup');
				optgroup.label = label;
				group.forEach((s) => optgroup.appendChild(option(String(s.id), s.title || '#' + s.id)));
				select.appendChild(optgroup);
			});
			if (value && !known.has(String(value))) select.appendChild(option(String(value), '#' + value));
			select.value = String(value || '');
			return select;
		}

		function rowView(row, i) {
			const tr = el('tr');
			const day = field('number', row.day, text.day);
			day.min = '1';
			day.max = String(dayMax);
			day.className = 'small-text';
			const name = field('text', row.name, text.place);
			const where = field('text', row.location, text.location);
			where.spellcheck = false;
			const reading = el('span', 'vl-geo-reading');
			const note = field('text', row.note, text.note);
			const story = storySelect(row.post);
			function check() {
				const result = geo.dayLine(geo.joinDay(row));
				const bad = typeof result === 'string';
				tr.classList.toggle('is-error', bad);
				reading.classList.toggle('is-error', bad);
				reading.textContent = bad ? (reasons[result] || '') : shown([result.lat, result.lon]);
			}
			[[day, 'day'], [name, 'name'], [where, 'location'], [note, 'note']].forEach(([input, key]) => {
				input.addEventListener('input', () => {
					row[key] = input.value;
					check();
					write();
				});
			});
			story.addEventListener('change', () => {
				row.post = story.value;
				write();
			});
			const up = button('↑', 'button button-small', text.moveUp);
			const down = button('↓', 'button button-small', text.moveDown);
			const remove = button('✕', 'button button-small', text.remove);
			up.disabled = i === 0;
			down.disabled = i === rows.length - 1;
			up.addEventListener('click', () => move(i, -1));
			down.addEventListener('click', () => move(i, 1));
			remove.addEventListener('click', () => {
				rows.splice(i, 1);
				render();
				write();
			});
			const cells = [[day], [name], [where, reading], [note], [story], [up, down, remove]];
			cells.forEach((items, c) => {
				const td = el('td', c === 5 ? 'vl-row-actions' : '');
				td.append(...items);
				tr.appendChild(td);
			});
			if (row.name || row.location || row.note || row.post) check();
			return tr;
		}
		function render() {
			body.replaceChildren(...rows.map(rowView));
		}
		function move(i, by) {
			const j = i + by;
			if (j < 0 || j >= rows.length) return;
			[rows[i], rows[j]] = [rows[j], rows[i]];
			render();
			write();
			const target = body.rows[j] && body.rows[j].querySelectorAll('.vl-row-actions button')[by < 0 ? 0 : 1];
			if (target && !target.disabled) target.focus();
		}
		function addStop(name, lat, lon, day) {
			const value = isDay(day) ? String(day) : lastDay();
			lastUsed = value;
			rows.push({ day: value, name: name, location: geo.format(lat, lon), note: '', post: '' });
			render();
			write();
		}

		addRow.addEventListener('click', () => {
			rows.push({ day: lastDay(), name: '', location: '', note: '', post: '' });
			render();
			body.rows[rows.length - 1].querySelectorAll('input')[1].focus();
		});
		addLinked.addEventListener('click', () => {
			const have = new Set(rows.map((r) => String(r.post)));
			let added = 0;
			(Array.isArray(data.linked) ? data.linked : []).forEach((s) => {
				if (have.has(String(s.id))) return;
				rows.push({ day: String(s.day), name: s.place, location: s.location, note: '', post: String(s.id) });
				have.add(String(s.id));
				added++;
			});
			if (added) {
				render();
				write();
			}
			status.textContent = added ? fill(text.addedLinked, added) : text.noLinked;
		});
		toggle.addEventListener('click', () => {
			asText = !asText;
			area.hidden = !asText;
			wrap.hidden = asText;
			addRow.hidden = asText;
			addLinked.hidden = asText;
			toggle.textContent = asText ? text.editRows : text.editText;
			if (!asText) {
				rows = parse();
				render();
			}
			(asText ? area : toggle).focus();
		});
		area.addEventListener('input', () => {
			rows = parse();
			if (pick) pick.redraw();
		});
		render();

		if (!root) return;
		const hooks = {
			points: () => rows.map((row) => {
				const point = geo.read(row.location);
				if (!Array.isArray(point)) return null;
				return {
					lat: point[0],
					lon: point[1],
					name: (isDay(row.day) ? fill(text.dayLabel, row.day) + ' · ' : '') + row.name,
					move: (lat, lon) => {
						row.location = geo.format(lat, lon);
						render();
						write();
					}
				};
			}).filter(Boolean),
			click: (lat, lon) => pick.ask(lat, lon),
			add: addStop,
			choose: (name, lat, lon) => addStop(name, lat, lon, lastDay()),
			lastDay: lastDay
		};
		pick = picker(root, hooks);
	}

	/* The block editor saves meta boxes in the background, so say after each save what couldn't be read. */
	function watchSaves() {
		const wp = window.wp;
		if (!wp || !wp.data || typeof wp.data.subscribe !== 'function') return;
		let editor = null;
		try {
			editor = wp.data.select('core/edit-post');
		} catch (e) {
			return;
		}
		if (!editor || typeof editor.isSavingMetaBoxes !== 'function') return;
		let saving = false;
		wp.data.subscribe(() => {
			const was = saving;
			saving = editor.isSavingMetaBoxes();
			if (was && !saving) afterSave(wp);
		});
	}
	function afterSave(wp) {
		const notices = wp.data.dispatch('core/notices');
		if (!notices) return;
		const messages = [];
		const input = document.getElementById('vl_location');
		const value = input ? input.value.trim() : '';
		const point = value ? geo.read(value) : null;
		if (typeof point === 'string') messages.push(fill(text.locationKept, value, reasons[point] || point));
		document.querySelectorAll('textarea.vl-geo-lines').forEach((area) => messages.push(...lineProblems(area)));
		if (messages.length) notices.createWarningNotice(text.savedProblems + ' ' + messages.join(' '), { id: 'vl-geo-problems' });
		else notices.removeNotice('vl-geo-problems');
	}

	function start() {
		document.querySelectorAll('textarea.vl-geo-lines').forEach((area) => {
			area.addEventListener('input', () => showProblems(area));
			showProblems(area);
		});
		postBox();
		tripBox();
		watchSaves();
	}
	if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start);
	else start();
})();
