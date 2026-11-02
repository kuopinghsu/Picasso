<?php
get_header();
?>
<main id="main" class="shell single-layout">
<?php
while ( have_posts() ) :
	the_post();
	$minutes = vl_read_minutes( get_the_content() );
	?>
	<article <?php post_class( 'article' ); ?>>
		<div class="article-intro">
			<span class="eyebrow"><?php echo esc_html( get_the_date() ); ?> · <?php the_category( ', ' ); ?></span>
			<h1><?php the_title(); ?></h1>
			<?php if ( has_excerpt() ) : ?>
				<p class="article-deck"><?php echo esc_html( get_the_excerpt() ); ?></p>
			<?php endif; ?>
			<?php /* translators: 1: author name, 2: minutes to read. */ ?>
			<div class="byline"><?php echo esc_html( sprintf( _n( 'By %1$s · %2$d min read', 'By %1$s · %2$d min read', $minutes, 'voyagelight' ), get_the_author(), $minutes ) ); ?></div>
		</div>
		<?php if ( has_post_thumbnail() ) : ?>
			<figure class="article-hero">
				<?php
				the_post_thumbnail( 'full' );
				$caption = get_the_post_thumbnail_caption();
				$camera  = vl_camera_line( get_post_thumbnail_id() );
				if ( $caption || $camera ) {
					echo '<figcaption>' . esc_html( $caption );
					if ( $camera ) {
						echo '<span class="vl-camera">' . esc_html( $camera ) . '</span>';
					}
					echo '</figcaption>';
				}
				?>
			</figure>
		<?php endif; ?>
		<div class="article-content">
			<?php
			the_content();
			wp_link_pages();
			?>
		</div>
		<?php
		$linked_trip = absint( get_post_meta( get_the_ID(), '_vl_trip_id', true ) );
		if ( $linked_trip && 'vl_trip' === get_post_type( $linked_trip ) && 'publish' === get_post_status( $linked_trip ) ) {
			echo '<div class="trip-backlink"><span class="eyebrow">' . esc_html__( 'PART OF A JOURNEY', 'voyagelight' ) . '</span><p><a href="' . esc_url( get_permalink( $linked_trip ) ) . '">← ' . esc_html( get_the_title( $linked_trip ) ) . '</a></p></div>';
		}
		echo vl_render_map( vl_post_points( get_the_ID() ), __( 'Places in this story', 'voyagelight' ), true ); // phpcs:ignore WordPress.Security.EscapeOutput -- escaped inside.
		?>
		<?php
		$details = array(
			array( __( 'Destination', 'voyagelight' ), get_the_term_list( get_the_ID(), 'destination', '', ' · ' ) ),
			array( __( 'Place type', 'voyagelight' ), get_the_term_list( get_the_ID(), 'travel_type', '', ' · ' ) ),
			array( __( 'Tags', 'voyagelight' ), get_the_tag_list( '', ' · ' ) ),
		);
		$details = array_filter(
			$details,
			function ( $detail ) {
				return is_string( $detail[1] ) && '' !== $detail[1];
			}
		);
		if ( $details ) {
			echo '<div class="article-meta">';
			foreach ( $details as $detail ) {
				echo '<div><strong>' . esc_html( $detail[0] ) . '</strong><p>' . wp_kses_post( $detail[1] ) . '</p></div>';
			}
			echo '</div>';
		}
		?>
		<div class="post-nav">
			<?php
			previous_post_link( '%link', esc_html__( '← Previous story', 'voyagelight' ) );
			next_post_link( '%link', esc_html__( 'Next story →', 'voyagelight' ) );
			?>
		</div>
		<?php
		if ( comments_open() || get_comments_number() ) {
			comments_template();
		}
		?>
	</article>
<?php endwhile; ?>
</main>
<?php
get_footer();
