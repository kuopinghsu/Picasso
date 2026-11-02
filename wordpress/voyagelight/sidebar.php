<aside class="sidebar">
	<div class="sidebar-panel"><h2><?php esc_html_e( 'Travel categories', 'voyagelight' ); ?></h2><ul><?php wp_list_categories( array( 'title_li' => '', 'hide_empty' => false ) ); ?></ul></div>
	<div class="sidebar-panel"><h2><?php esc_html_e( 'Destinations', 'voyagelight' ); ?></h2><ul><?php wp_list_categories( array( 'taxonomy' => 'destination', 'title_li' => '', 'hide_empty' => false ) ); ?></ul></div>
	<div class="sidebar-panel"><h2><?php esc_html_e( 'Place types', 'voyagelight' ); ?></h2><ul><?php wp_list_categories( array( 'taxonomy' => 'travel_type', 'title_li' => '', 'hide_empty' => false ) ); ?></ul></div>
	<?php
	$journeys = get_posts(
		array(
			'post_type'      => 'vl_trip',
			'post_status'    => 'publish',
			'posts_per_page' => -1,
			'orderby'        => array(
				'title' => 'ASC',
				'ID'    => 'ASC',
			),
		)
	);
	if ( $journeys ) {
		echo '<div class="sidebar-panel"><h2>' . esc_html__( 'Journeys', 'voyagelight' ) . '</h2><ul>';
		foreach ( $journeys as $journey ) {
			echo '<li><a href="' . esc_url( get_permalink( $journey ) ) . '">' . esc_html( get_the_title( $journey ) ) . '</a></li>';
		}
		echo '</ul></div>';
	}
	?>
</aside>
