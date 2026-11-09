/* Moves a start page section up or down in the Customizer's list, then saves the new order as comma-separated keys. */
(function () {
	'use strict';
	document.addEventListener('click', function (event) {
		var button = event.target.closest('.vl-order button');
		if (!button) {
			return;
		}
		var item = button.closest('li');
		var list = item.parentNode;
		if (button.classList.contains('vl-order-up') && item.previousElementSibling) {
			list.insertBefore(item, item.previousElementSibling);
		} else if (button.classList.contains('vl-order-down') && item.nextElementSibling) {
			list.insertBefore(item.nextElementSibling, item);
		} else {
			return;
		}
		// Moving the item can drop focus; keep it on the button so the keyboard can press it again.
		button.focus();
		var keys = Array.prototype.map.call(list.children, function (child) {
			return child.getAttribute('data-key');
		});
		wp.customize(list.getAttribute('data-setting')).set(keys.join(','));
	});
}());
