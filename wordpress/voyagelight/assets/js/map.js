(function () {
	'use strict';
	var text = window.vlText || {};
	var loading = null;
	function asset(tag, attrs) {
		return new Promise(function (resolve, reject) {
			var el = document.createElement(tag);
			Object.keys(attrs).forEach(function (k) { el[k] = attrs[k]; });
			el.onload = resolve;
			el.onerror = reject;
			document.head.appendChild(el);
		});
	}
	function leaflet() {
		if (window.L) return Promise.resolve();
		if (loading) return loading;
		loading = Promise.all([
			asset('link', { rel: 'stylesheet', href: 'https://unpkg.com/leaflet@1.9.4/dist/leaflet.css', integrity: 'sha256-p4NxAoJBhIIN+hmNHrzRCf9tD/miZyoHS5obTRR9BMY=', crossOrigin: 'anonymous' }),
			asset('script', { src: 'https://unpkg.com/leaflet@1.9.4/dist/leaflet.js', integrity: 'sha256-20nQCchB9co0qIjJZRGuk2/Z9VM+kNiyxNV1lvTlZBo=', crossOrigin: 'anonymous' })
		]).catch(function (error) {
			loading = null;
			throw error;
		});
		return loading;
	}
	document.addEventListener('click', function (e) {
		var button = e.target.closest('.vl-map-load');
		if (!button) return;
		var wrap = button.closest('.vl-map');
		var raw = wrap && wrap.getAttribute('data-map');
		if (!raw) return;
		var data;
		try { data = JSON.parse(raw); } catch (ex) { return; }
		button.disabled = true;
		button.textContent = text.loadingMap;
		leaflet().then(function () {
			wrap.innerHTML = '';
			var canvas = document.createElement('div');
			canvas.className = 'vl-map-canvas';
			wrap.appendChild(canvas);
			var view = L.map(canvas, { scrollWheelZoom: false });
			L.tileLayer('https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png', { maxZoom: 18, attribution: '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors' }).addTo(view);
			var bounds = [];
			data.points.forEach(function (p) {
				if (typeof p.lat !== 'number' || typeof p.lon !== 'number') return;
				var loc = [p.lat, p.lon];
				bounds.push(loc);
				var popup = document.createElement('div');
				var strong = document.createElement('strong');
				strong.textContent = p.name || text.place;
				popup.appendChild(strong);
				if (p.note) {
					var desc = document.createElement('p');
					desc.textContent = p.note;
					popup.appendChild(desc);
				}
				if (p.url && /^https?:\/\//.test(p.url)) {
					var a = document.createElement('a');
					a.href = p.url;
					a.textContent = text.readStory;
					popup.appendChild(a);
				}
				L.marker(loc).addTo(view).bindPopup(popup);
			});
			if (data.route && bounds.length > 1) L.polyline(bounds, { color: '#358ba5', weight: 3, dashArray: '7 8' }).addTo(view);
			if (bounds.length === 1) view.setView(bounds[0], 9);
			else if (bounds.length) view.fitBounds(bounds, { padding: [35, 35], maxZoom: 12 });
			else view.setView([20, 0], 2);
			setTimeout(function () { view.invalidateSize(); }, 100);
		}).catch(function () {
			button.disabled = false;
			button.textContent = text.retryMap;
			var err = wrap.querySelector('.vl-map-error');
			if (!err) {
				err = document.createElement('p');
				err.className = 'vl-map-error';
				wrap.appendChild(err);
			}
			err.textContent = text.mapFailed;
		});
	});
})();
