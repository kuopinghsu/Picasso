<?php
if ( post_password_required() ) {
	return;
}
?>
<section class="comments">
	<h2><?php esc_html_e( 'Discussion', 'voyagelight' ); ?></h2>
	<?php
	if ( have_comments() ) {
		echo '<ol class="comment-list">';
		wp_list_comments( array( 'style' => 'ol' ) );
		echo '</ol>';
		the_comments_pagination();
	}
	comment_form();
	?>
</section>
