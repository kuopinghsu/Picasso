<?php
get_header();
?>
<main id="main" class="shell page-main">
<?php
while ( have_posts() ) :
	the_post();
	?>
	<article <?php post_class( 'article' ); ?>>
		<div class="article-intro"><h1><?php the_title(); ?></h1></div>
		<div class="article-content">
			<?php
			the_content();
			wp_link_pages();
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
