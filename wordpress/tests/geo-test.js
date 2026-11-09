// Checks assets/js/geo.js, the theme's and the Picasso Places plugin's copy,
// against geo-cases.json, the cases geo-test.php checks inc/geo.php against.
//
//     node wordpress/tests/geo-test.js
'use strict';
const path = require('path');
const cases = require(path.join(__dirname, 'geo-cases.json'));

let failed = 0;
let passed = 0;
let reader = '';

function same(expected, actual) {
	if (expected && actual && typeof expected === 'object' && typeof actual === 'object') {
		const keys = Object.keys(expected);
		if (keys.join() !== Object.keys(actual).join()) return false;
		return keys.every((k) => same(expected[k], actual[k]));
	}
	if (typeof expected === 'number' && typeof actual === 'number') return Math.abs(expected - actual) < 2e-6;
	return expected === actual;
}

function check(name, input, expected, actual) {
	if (same(expected, actual)) {
		passed++;
		return;
	}
	failed++;
	console.error('FAIL ' + reader + ' ' + name + ' ' + JSON.stringify(input) + '\n  expected ' + JSON.stringify(expected) + '\n  got      ' + JSON.stringify(actual));
}

for (const folder of ['voyagelight', 'picasso-places']) {
	reader = folder;
	const geo = require(path.join(__dirname, '..', folder, 'assets', 'js', 'geo.js'));
	const problems = (text, kind, max) => {
		const out = [];
		text.split(/\r\n|\r|\n/).slice(0, max).forEach((line, i) => {
			const row = kind === 'day' ? geo.dayLine(line) : geo.stopLine(line);
			if (typeof row === 'string') out.push([i + 1, line.trim(), row]);
		});
		return out;
	};

	cases.location.forEach(([input, expected]) => check('location', input, expected, geo.read(input)));
	cases.stop.forEach(([input, expected]) => check('stop', input, expected, geo.stopLine(input)));
	cases.day.forEach(([input, expected]) => check('day', input, expected, geo.dayLine(input)));
	cases.problems.forEach(([input, kind, expected]) => check('problems', input, expected, problems(input, kind, 200)));
	check('unescape', '', '39.8628,-4.0273 100%25', geo.unescape('39.8628%2C-4.0273 100%25'));
	check('valid coord', '90.5', null, geo.validCoord('90.5', 'lat'));
	check('split old stop', '', { name: 'A', location: '1, 2', note: 'n' }, geo.splitStop('A | 1 | 2 | n'));
	check('split old day', '', { day: '1', name: 'A', location: '1, 2', note: 'n', post: '5' }, geo.splitDay('1 | A | 1 | 2 | n | 5'));
	check('join day', '', '2 | A | 1, 2 |  | 5', geo.joinDay({ day: 2, name: 'A', location: '1, 2', note: '', post: '5' }));
	check('join day trims', '', '2 | A/B | 1, 2', geo.joinDay({ day: 2, name: 'A|B', location: '1, 2', note: '', post: '' }));
	check('join stop', '', 'A | 1, 2', geo.joinStop({ name: 'A', location: '1, 2', note: '' }));
	check('format', '', '39.862806, -4.027306', geo.format(39.8628055555, -4.0273055555));
	check('format small', '', '0.00001, 30', geo.format(0.00001, 30));
	check('split day note with |', '', { day: '1', name: 'A', location: '1, 2', note: 'n | m', post: '5' }, geo.splitDay('1 | A | 1, 2 | n | m | 5'));
}

console.log(passed + ' passed, ' + failed + ' failed');
process.exit(failed ? 1 : 0);
