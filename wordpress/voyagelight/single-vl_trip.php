<?php
get_header();
?>
<main id="main" class="shell single-layout journey-page">
<?php
while ( have_posts() ) :
	the_post();
	$trip_id    = get_the_ID();
	$days       = vl_trip_parse_days( get_post_meta( $trip_id, '_vl_trip_days', true ) );
	$points     = vl_trip_points( $trip_id );
	$stories    = vl_trip_stories( $trip_id );
	$daynumbers = array_values( array_unique( array_column( $days, 'day' ) ) );
	$day_count  = count( $daynumbers );
	$places     = count( $points );
	$story_count = count( $stories );
	?>
	<article class="article">
		<div class="article-intro">
			<?php echo vl_breadcrumbs(); // phpcs:ignore WordPress.Security.EscapeOutput.OutputNotEscaped -- escaped in vl_breadcrumbs(). ?>
			<span class="eyebrow"><?php esc_html_e( 'Journey', 'voyagelight' ); ?></span>
			<h1><?php the_title(); ?></h1>
			<?php if ( has_excerpt() ) : ?>
				<p class="article-deck"><?php echo esc_html( get_the_excerpt() ); ?></p>
			<?php endif; ?>
			<div class="journey-stats">
				<?php /* translators: %d: number of days in a journey. */ ?>
				<span><?php echo esc_html( sprintf( _n( '%d day', '%d days', $day_count, 'voyagelight' ), $day_count ) ); ?></span>
				<?php /* translators: %d: number of places in a journey. */ ?>
				<span><?php echo esc_html( sprintf( _n( '%d place', '%d places', $places, 'voyagelight' ), $places ) ); ?></span>
				<?php /* translators: %d: number of stories linked to a journey. */ ?>
				<span><?php echo esc_html( sprintf( _n( '%d story', '%d stories', $story_count, 'voyagelight' ), $story_count ) ); ?></span>
			</div>
		</div>
		<?php if ( has_post_thumbnail() ) : ?>
			<figure class="article-hero"><?php the_post_thumbnail( 'full' ); ?></figure>
		<?php endif; ?>
		<div class="article-content"><?php the_content(); ?></div>

		<?php
		// A password-protected Journey keeps its places and stories hidden until the password is entered.
		if ( ! post_password_required() ) :
			?>
		<?php if ( $days ) : ?>
		<section class="vl-trip-explorer" data-itinerary="<?php echo esc_attr( wp_json_encode( vl_trip_explorer_stops( $days ), JSON_HEX_TAG | JSON_HEX_APOS | JSON_HEX_AMP | JSON_HEX_QUOT | JSON_UNESCAPED_UNICODE ) ); ?>">
			<div class="section-heading">
				<div>
					<span class="eyebrow"><?php esc_html_e( 'Journey map', 'voyagelight' ); ?></span>
					<h2><?php esc_html_e( 'Daily stops & map', 'voyagelight' ); ?></h2>
					<p><?php esc_html_e( 'See where each day’s stops are in relation to each other. Dashed lines show the order of visits, not a route to follow.', 'voyagelight' ); ?></p>
				</div>
			</div>
			<div class="vl-day-tabs" role="group" aria-label="<?php esc_attr_e( 'Select journey day', 'voyagelight' ); ?>">
				<?php // All days first: it is the one shown when the page opens. ?>
				<button type="button" class="vl-day-tab" data-day="all" aria-pressed="true"><?php esc_html_e( 'All days', 'voyagelight' ); ?></button>
				<?php foreach ( $daynumbers as $day ) : ?>
					<button type="button" class="vl-day-tab" data-day="<?php echo esc_attr( $day ); ?>" aria-pressed="false"><?php echo esc_html( vl_day_label( $day ) ); ?></button>
				<?php endforeach; ?>
			</div>
			<div class="vl-trip-explorer-grid">
				<div class="vl-trip-map" role="region" aria-label="<?php esc_attr_e( 'Journey map', 'voyagelight' ); ?>">
					<div class="vl-trip-map-placeholder">
						<?php echo vl_pin_icon(); // phpcs:ignore WordPress.Security.EscapeOutput.OutputNotEscaped -- fixed markup. ?>
						<p><?php esc_html_e( 'Interactive journey map', 'voyagelight' ); ?></p>
						<button class="vl-trip-load" type="button"><?php esc_html_e( 'Load map', 'voyagelight' ); ?></button>
						<small><?php esc_html_e( 'Uses map tiles from OpenStreetMap. Loading shares your IP address with it.', 'voyagelight' ); ?></small>
					</div>
				</div>
				<div class="vl-trip-stop-panel"><h3 class="vl-day-heading"></h3><ol class="vl-day-stop-list"></ol></div>
			</div>
		</section>
		<?php endif; ?>

		<section class="journey-timeline">
			<div class="section-heading">
				<div>
					<span class="eyebrow"><?php esc_html_e( 'Day by day', 'voyagelight' ); ?></span>
					<h2><?php esc_html_e( 'Itinerary', 'voyagelight' ); ?></h2>
					<p><?php esc_html_e( 'Stops and notes in order. Select a linked story to read more.', 'voyagelight' ); ?></p>
				</div>
			</div>
			<?php
			if ( $days ) {
				$current = 0;
				foreach ( $days as $item ) {
					if ( $current !== $item['day'] ) {
						if ( $current ) {
							echo '</ol></section>';
						}
						$current = $item['day'];
						echo '<section class="journey-day"><h3>' . esc_html( vl_day_label( $current ) ) . '</h3><ol>';
					}
					echo '<li><strong>' . esc_html( $item['name'] ) . '</strong>';
					if ( $item['note'] ) {
						echo '<p>' . esc_html( $item['note'] ) . '</p>';
					}
					if ( $item['post_id'] && 'post' === get_post_type( $item['post_id'] ) && 'publish' === get_post_status( $item['post_id'] ) ) {
						echo '<a href="' . esc_url( get_permalink( $item['post_id'] ) ) . '">' . esc_html__( 'Read story ↗', 'voyagelight' ) . '</a>';
					}
					echo '</li>';
				}
				if ( $current ) {
					echo '</ol></section>';
				}
			} else {
				echo '<p class="map-empty">' . esc_html__( 'No itinerary stops yet. Add Day | Place | location in the Journey editor.', 'voyagelight' ) . '</p>';
			}
			?>
		</section>

		<section class="journey-related">
			<div class="section-heading">
				<div>
					<span class="eyebrow"><?php esc_html_e( 'Travel journal', 'voyagelight' ); ?></span>
					<h2><?php esc_html_e( 'Stories from this journey', 'voyagelight' ); ?></h2>
				</div>
			</div>
			<?php
			if ( $stories ) {
				echo '<div class="listing-grid">';
				foreach ( $stories as $story ) {
					vl_card( $story->ID );
				}
				echo '</div>';
			} else {
				echo '<p>' . esc_html__( 'No linked travel stories yet. Choose this journey in a story’s editor.', 'voyagelight' ) . '</p>';
			}
			?>
		</section>
		<?php endif; ?>
	</article>
<?php endwhile; ?>
</main>
<?php
get_footer();
