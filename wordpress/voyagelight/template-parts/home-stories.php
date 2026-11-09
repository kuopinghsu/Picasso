<?php
/**
 * The start page's stories section; front-page.php draws the sections in the order set in the Customizer.
 *
 * @package VoyageLight
 */

$stories_url = $args['stories_url'];

// The widgets from Appearance > Widgets > Start page sidebar, or else the travel categories.
// None when the sidebar is turned off in Appearance > Customize > VoyageLight stories.
$home_shown     = 'none' !== vl_sidebar_position();
$home_widgets   = $home_shown && is_active_sidebar( 'home-1' );
$category_panel = $home_shown && ! $home_widgets ? vl_category_panel() : '';
$home_side      = $home_widgets || '' !== $category_panel;
?>
<section id="stories" class="shell section<?php echo $home_side ? ' content-layout home-stories' : ''; ?>">
	<?php if ( $home_side ) : ?>
	<?php // First in the page, so on a phone the categories come before the stories, as a row of chips; widgets move after them. ?>
	<aside class="sidebar<?php echo $home_widgets ? ' has-widgets' : ''; ?>">
		<?php
		if ( $home_widgets ) {
			dynamic_sidebar( 'home-1' );
		} else {
			echo $category_panel; // phpcs:ignore WordPress.Security.EscapeOutput.OutputNotEscaped -- escaped in vl_category_panel().
		}
		?>
	</aside>
	<div class="content-main">
	<?php endif; ?>
	<div class="section-heading">
		<div>
			<span class="eyebrow"><?php esc_html_e( 'Travel journal', 'voyagelight' ); ?></span>
			<h2><?php esc_html_e( 'Latest travel stories', 'voyagelight' ); ?></h2>
			<p><?php esc_html_e( 'Places, people, and the moments worth remembering.', 'voyagelight' ); ?></p>
		</div>
		<a class="section-more" href="<?php echo esc_url( $stories_url ); ?>"><?php esc_html_e( 'All stories ↗', 'voyagelight' ); ?></a>
	</div>
	<div class="story-grid">
		<?php
		$latest = new WP_Query(
			array(
				'posts_per_page'      => vl_home_count( 'stories', 5 ),
				'ignore_sticky_posts' => true,
				'orderby'             => array(
					'date' => 'DESC',
					'ID'   => 'DESC',
				),
			)
		);
		if ( $latest->have_posts() ) {
			// The newest story is the large card only when it has a photo; otherwise all the cards are the same size.
			$lead = has_post_thumbnail( $latest->posts[0] );
			while ( $latest->have_posts() ) {
				$latest->the_post();
				vl_card( get_the_ID(), $lead && 0 === $latest->current_post );
			}
			wp_reset_postdata();
		} else {
			$samples = array(
				array( 'greenland', __( 'Eqi Glacier: A Close Encounter with the Icefjord', 'voyagelight' ), __( 'Greenland', 'voyagelight' ), __( 'Travel stories', 'voyagelight' ) ),
				array( 'nordic', __( 'Under the Northern Lights', 'voyagelight' ), __( 'Nordic', 'voyagelight' ), __( 'Nature & Scenery', 'voyagelight' ) ),
				array( 'spain', __( 'Toledo: A City of History and Light', 'voyagelight' ), __( 'Spain', 'voyagelight' ), __( 'Culture', 'voyagelight' ) ),
				array( 'japan', __( 'Tokyo: A Walk Through Spring', 'voyagelight' ), __( 'Japan', 'voyagelight' ), __( 'City Walks', 'voyagelight' ) ),
				array( 'baltic', __( 'A Morning in the Baltic Old Town', 'voyagelight' ), __( 'Baltic', 'voyagelight' ), __( 'History & Culture', 'voyagelight' ) ),
			);
			foreach ( $samples as $i => $sample ) {
				echo '<article class="story-card' . ( 0 === $i ? ' is-lead' : '' ) . '"><div class="story-visual"><img src="' . esc_url( vl_image( $sample[0] ) ) . '" alt="' . esc_attr( $sample[1] ) . '"><span class="story-badge">' . esc_html( $sample[2] ) . '</span></div>';
				echo '<div class="story-copy"><span class="eyebrow">' . esc_html( $sample[3] ) . ' · ' . esc_html__( 'Sample', 'voyagelight' ) . '</span><h3>' . esc_html( $sample[1] ) . '</h3>';
				echo '<p>' . esc_html__( 'Demo illustration. Publish your own story to replace this sample.', 'voyagelight' ) . '</p></div></article>';
			}
		}
		?>
	</div>
	<?php if ( $home_side ) : ?>
	</div>
	<?php endif; ?>
</section>
