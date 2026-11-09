(function () {
	'use strict';
	const text = window.vlText || {};
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
			add('link', { rel: 'stylesheet', href: text.leaflet + 'leaflet.css' }),
			add('script', { src: text.leaflet + 'leaflet.js' })
		]).catch((error) => {
			loader = null;
			throw error;
		});
		return loader;
	}
	const dayLabel = (day) => (text.day || 'Day %s').replace('%s', day);
	function storyLink(url) {
		if (!url || !text.readStory || !/^https?:\/\//.test(url)) return null;
		const a = document.createElement('a');
		a.href = url;
		a.textContent = text.readStory;
		return a;
	}

	document.querySelectorAll('.vl-trip-explorer').forEach((el) => {
		let items = [];
		try { items = JSON.parse(el.dataset.itinerary || '[]'); } catch (e) {}
		const tabs = [...el.querySelectorAll('.vl-day-tab')];
		const heading = el.querySelector('.vl-day-heading');
		const list = el.querySelector('.vl-day-stop-list');
		const host = el.querySelector('.vl-trip-map');
		let selected = 'all';
		let map = null;
		let overlays = [];

		function subset() {
			return selected === 'all' ? items : items.filter((p) => String(p.day) === selected);
		}
		function render() {
			tabs.forEach((b) => {
				const on = b.dataset.day === selected;
				b.setAttribute('aria-pressed', String(on));
				b.classList.toggle('active', on);
			});
			heading.textContent = selected === 'all' ? text.allStops : dayLabel(selected);
			list.replaceChildren();
			subset().forEach((p) => {
				const li = document.createElement('li');
				const name = document.createElement('strong');
				name.textContent = p.name;
				li.appendChild(name);
				if (p.note) {
					const note = document.createElement('p');
					note.textContent = p.note;
					li.appendChild(note);
				}
				const link = storyLink(p.url);
				if (link) li.appendChild(link);
				list.appendChild(li);
			});
			if (map) updateMap();
		}
		function updateMap() {
			overlays.forEach((x) => map.removeLayer(x));
			overlays = [];
			const bounds = [];
			subset().forEach((p, i) => {
				const point = [Number(p.lat), Number(p.lon)];
				if (!Number.isFinite(point[0]) || !Number.isFinite(point[1])) return;
				bounds.push(point);
				const pop = document.createElement('div');
				const title = document.createElement('strong');
				title.textContent = (i + 1) + '. ' + p.name;
				pop.appendChild(title);
				if (p.note) {
					const desc = document.createElement('p');
					desc.textContent = p.note;
					pop.appendChild(desc);
				}
				const link = storyLink(p.url);
				if (link) pop.appendChild(link);
				overlays.push(L.marker(point).addTo(map).bindPopup(pop));
			});
			if (bounds.length > 1) {
				overlays.push(L.polyline(bounds, { color: '#087785', weight: 3, dashArray: '6 9' }).addTo(map));
				map.fitBounds(bounds, { padding: [35, 35], maxZoom: 11 });
			} else if (bounds.length) {
				map.setView(bounds[0], 10);
			} else {
				map.setView([20, 0], 2);
			}
			setTimeout(() => map.invalidateSize(), 70);
		}

		tabs.forEach((b) => b.addEventListener('click', () => { selected = b.dataset.day; render(); }));
		const load = el.querySelector('.vl-trip-load');
		if (load) {
			load.addEventListener('click', () => {
				// The button goes with the placeholder; a keyboard user who pressed it lands on the map, which Leaflet
				// makes focusable. Asked now: disabling the button below takes the focus off it.
				const hadFocus = document.activeElement === load;
				load.disabled = true;
				load.textContent = text.loadingMap;
				leaflet().then(() => {
					host.replaceChildren();
					const canvas = document.createElement('div');
					canvas.className = 'vl-trip-map-canvas';
					host.appendChild(canvas);
					map = L.map(canvas, { scrollWheelZoom: false });
					L.tileLayer('https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png', { maxZoom: 18, attribution: '© OpenStreetMap contributors' }).addTo(map);
					updateMap();
					if (hadFocus) map.getContainer().focus({ preventScroll: true });
				}).catch(() => {
					load.disabled = false;
					load.textContent = text.retryMap;
					let msg = host.querySelector('.vl-trip-map-error');
					if (!msg) {
						msg = document.createElement('p');
						msg.className = 'vl-trip-map-error';
						host.appendChild(msg);
					}
					msg.textContent = text.mapFailed;
				});
			});
		}
		render();
	});
})();
