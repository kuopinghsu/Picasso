/*
 * Reads locations the way inc/geo.php does; change both together.
 * wordpress/tests/geo-test.php and geo-test.js check them against the same cases.
 */
(function (root) {
	'use strict';
	const N = '([+-]?\\d+(?:\\.\\d+)?)';
	const PAIR = N + '\\s*,\\s*\\+?\\s*' + N;
	const CORE = '([+-]?)\\s*([0-9]+(?:\\.[0-9]+)?)(?:\\s*°)?(?:\\s*([0-9]+(?:\\.[0-9]+)?)\\s*\')?(?:\\s*([0-9]+(?:\\.[0-9]+)?)\\s*")?';
	const SEP = '(?:\\s*[,;]\\s*|\\s+|(?<![0-9.])|(?![0-9.]))';
	const PLAIN = new RegExp('^' + CORE + SEP + CORE + '$', 'u');
	const PREFIX = new RegExp('^([NSEW])\\s*' + CORE + SEP + '([NSEW])\\s*' + CORE + '$', 'u');
	const SUFFIX = new RegExp('^' + CORE + '\\s*([NSEW])' + SEP + CORE + '\\s*([NSEW])$', 'u');
	const SHORT = /^(?:https?:\/\/)?(?:www\.)?(?:maps\.app\.goo\.gl|goo\.gl|g\.co|g\.page|apple\.co|maps\.apple)\/|^(?:https?:\/\/)?(?:www\.)?(?:osm|openstreetmap)\.org\/go\//i;
	const LINK = /^(?:[a-z][a-z0-9+.-]*:|www\.)|\b(?:maps|openstreetmap|osm)\.[a-z]/i;
	const LINKS = [
		new RegExp('@' + PAIR),
		new RegExp('[?&](?:q|query)=(?:loc:)?\\s*' + PAIR, 'i'),
		new RegExp('[?&](?:ll|coordinate|center)=' + PAIR, 'i'),
		new RegExp('/maps/(?:search|place)/' + PAIR, 'i'),
		new RegExp('#map=\\d+(?:\\.\\d+)?/' + N + '/' + N),
		new RegExp('^geo:' + PAIR, 'i')
	];
	const PRIMES = { '′': "'", '’': "'", '‘': "'", '´': "'", '″': '"', '”': '"', '“': '"', 'º': '°', '˚': '°', '，': ',', '；': ';' };

	// The same bound as VL_GEO_DAY_MAX in inc/geo.php.
	const DAY_MAX = 9999;

	const isNumber = (text) => /^[+-]?\d+(?:\.\d+)?$/.test(text);

	function validCoord(value, kind) {
		const number = typeof value === 'number' ? value : (typeof value === 'string' && value.trim() !== '' ? Number(value) : NaN);
		const limit = kind === 'lat' ? 90 : 180;
		return Number.isFinite(number) && number >= -limit && number <= limit ? number : null;
	}

	function unescape(text) {
		return String(text).replace(/%([2-7][0-9a-f])/gi, (match, hex) => {
			const char = String.fromCharCode(parseInt(hex, 16));
			return char === '%' || char === '\x7f' ? match : char;
		});
	}

	function dms(sign, degrees, minutes, seconds, hemisphere) {
		minutes = minutes || '';
		seconds = seconds || '';
		if ((minutes !== '' && degrees.includes('.')) || (seconds !== '' && (minutes === '' || minutes.includes('.')))
			|| (parseFloat(minutes) || 0) >= 60 || (parseFloat(seconds) || 0) >= 60 || (sign === '-' && hemisphere !== '')) {
			return null;
		}
		const value = parseFloat(degrees) + (parseFloat(minutes) || 0) / 60 + (parseFloat(seconds) || 0) / 3600;
		return sign === '-' || hemisphere === 'S' || hemisphere === 'W' ? -value : value;
	}

	function textPair(text) {
		text = text.replace(/[′’‘´″”“º˚，；]/g, (c) => PRIMES[c]).replace(/''/g, '"').replace(/[a-z]/g, (c) => c.toUpperCase());
		let m = PLAIN.exec(text);
		if (m) {
			const first = dms(m[1], m[2], m[3], m[4], '');
			const second = dms(m[5], m[6], m[7], m[8], '');
			return first === null || second === null ? null : [first, second];
		}
		let parts;
		if ((m = PREFIX.exec(text))) {
			parts = [[m[1], m[2], m[3], m[4], m[5]], [m[6], m[7], m[8], m[9], m[10]]];
		} else if ((m = SUFFIX.exec(text))) {
			parts = [[m[5], m[1], m[2], m[3], m[4]], [m[10], m[6], m[7], m[8], m[9]]];
		} else {
			return null;
		}
		const values = {};
		parts.forEach((p) => { values[p[0]] = dms(p[1], p[2], p[3], p[4], p[0]); });
		const pick = (a, b) => (a in values && values[a] !== null ? values[a] : (b in values && values[b] !== null ? values[b] : null));
		const lat = pick('N', 'S');
		const lon = pick('E', 'W');
		return lat === null || lon === null || Object.keys(values).length !== 2 ? null : [lat, lon];
	}

	function linkPair(text) {
		let m = /!3d([+-]?\d+(?:\.\d+)?)!4d([+-]?\d+(?:\.\d+)?)/.exec(text);
		if (m) return [parseFloat(m[1]), parseFloat(m[2])];
		const a = new RegExp('[?&#]mlat=' + N, 'i').exec(text);
		const b = new RegExp('[?&#]mlon=' + N, 'i').exec(text);
		if (a && b) return [parseFloat(a[1]), parseFloat(b[1])];
		for (const pattern of LINKS) {
			m = pattern.exec(text);
			if (m) return [parseFloat(m[1]), parseFloat(m[2])];
		}
		m = /[?&](?:q|query)=([^&#]+)/i.exec(text);
		return m ? textPair(m[1].replace(/\+/g, ' ').trim()) : null;
	}

	/* [latitude, longitude] rounded to six decimals, or why it can't be read. */
	function read(text) {
		text = unescape(text).replace(/[\u00a0\u202f]/g, ' ').replace(/^[ \t\n\r\0\x0B]+|[ \t\n\r\0\x0B]+$/g, '');
		if (text === '') return 'empty';
		if (text.length > 2048) return 'unreadable';
		if (SHORT.test(text)) return 'short-link';
		let pair;
		if (LINK.test(text)) {
			pair = linkPair(text);
			if (pair === null) return 'link';
		} else {
			pair = textPair(text);
			if (pair === null) return 'unreadable';
		}
		const lat = validCoord(pair[0], 'lat');
		const lon = validCoord(pair[1], 'lon');
		if (lat === null || lon === null) return 'range';
		return [round(lat), round(lon)];
	}

	const round = (value) => Math.round(value * 1e6) / 1e6;
	const columns = (line) => String(line).trim().split('|').map((c) => c.trim());

	function splitStop(line) {
		const cols = columns(line);
		if (cols.length >= 3 && isNumber(cols[1]) && isNumber(cols[2])) {
			return { name: cols[0], location: cols[1] + ', ' + cols[2], note: cols.slice(3).join(' | ') };
		}
		return { name: cols[0], location: cols[1] || '', note: cols.slice(2).join(' | ') };
	}

	function splitDay(line) {
		const cols = columns(line);
		if (cols.length === 1) return { day: '', name: cols[0], location: '', note: '', post: '' };
		while (cols.length < 3) cols.push('');
		let where = cols[2];
		let rest = cols.slice(3);
		if (cols.length >= 4 && isNumber(cols[2]) && isNumber(cols[3])) {
			where = cols[2] + ', ' + cols[3];
			rest = cols.slice(4);
		}
		// A note can hold a "|", so the post ID is the last column after it, when that is a number or empty.
		const post = rest.length >= 2 && /^\d*$/.test(rest[rest.length - 1]) ? rest.pop() : '';
		return { day: cols[0], name: cols[1], location: where, note: rest.join(' | '), post };
	}

	function stopLine(line) {
		if (String(line).trim() === '') return null;
		const stop = splitStop(line);
		if (stop.name === '') return 'name';
		const point = read(stop.location);
		if (!Array.isArray(point)) return point;
		return { name: stop.name, lat: point[0], lon: point[1], note: stop.note };
	}

	function dayLine(line) {
		if (String(line).trim() === '') return null;
		const cols = columns(line);
		const row = cols.length === 1 ? { day: cols[0], name: '', location: '' } : splitDay(line);
		// DAY_MAX matches VL_GEO_DAY_MAX; scripts/build-theme.py checks the two.
		// The editor passes that constant as vlGeoAdmin.dayMax.
		const limit = typeof vlGeoAdmin !== 'undefined' && vlGeoAdmin && +vlGeoAdmin.dayMax >= 1 ? +vlGeoAdmin.dayMax : DAY_MAX;
		if (!/^\d+$/.test(row.day) || +row.day < 1 || +row.day > limit) return 'day';
		if (row.name === '') return 'name';
		const point = read(row.location);
		if (!Array.isArray(point)) return point;
		return { day: +row.day, name: row.name, lat: point[0], lon: point[1], note: row.note, post_id: /^\d+$/.test(row.post) ? +row.post : 0 };
	}

	const cell = (value) => String(value === undefined || value === null ? '' : value).replace(/\|/g, '/').replace(/[\r\n\t]+/g, ' ').trim();

	function join(values, keep) {
		const cols = values.map(cell);
		while (cols.length > keep && cols[cols.length - 1] === '') cols.pop();
		return cols.join(' | ').replace(/\s+$/, '');
	}

	const joinStop = (s) => join([s.name, s.location, s.note], 2);
	const joinDay = (r) => join([r.day, r.name, r.location, r.note, r.post], 3);
	const format = (lat, lon) => round(lat) + ', ' + round(lon);

	const api = { read, isNumber, validCoord, unescape, stopLine, dayLine, splitStop, splitDay, joinStop, joinDay, format, dayMax: DAY_MAX };
	if (typeof module === 'object' && module.exports) {
		module.exports = api;
	} else {
		root.vlGeo = api;
	}
})(this);
