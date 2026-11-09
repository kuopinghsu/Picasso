<aside class="sidebar">
	<?php
	// The widgets from Appearance > Widgets > Sidebar, or the theme's own panels while it has none.
	if ( is_active_sidebar( 'sidebar-1' ) ) {
		dynamic_sidebar( 'sidebar-1' );
	} else {
		foreach ( VL_Panel_Widget::panels() as $vl_panel => $vl_heading ) {
			$vl_list = vl_panel_list( $vl_panel );
			if ( '' !== $vl_list ) {
				echo '<div class="sidebar-panel"><h2>' . esc_html( $vl_heading ) . '</h2>' . $vl_list . '</div>'; // phpcs:ignore WordPress.Security.EscapeOutput.OutputNotEscaped -- escaped in vl_panel_list().
			}
		}
	}
	?>
</aside>
