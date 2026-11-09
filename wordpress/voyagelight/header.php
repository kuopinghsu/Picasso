<?php
if ( ! defined( 'ABSPATH' ) ) {
	exit;
}
?><!doctype html>
<html <?php language_attributes(); ?>>
<head>
<meta charset="<?php bloginfo( 'charset' ); ?>">
<meta name="viewport" content="width=device-width,initial-scale=1">
<?php wp_head(); ?>
</head>
<body <?php body_class(); ?>>
<?php wp_body_open(); ?>
<a href="#main" class="skip-link"><?php esc_html_e( 'Skip to content', 'voyagelight' ); ?></a>
<header class="site-header">
	<div class="shell header-inner">
		<a class="brand" href="<?php echo esc_url( home_url( '/' ) ); ?>" aria-label="<?php echo esc_attr( get_bloginfo( 'name' ) ); ?>">
			<?php
			// The logo chosen in Appearance > Customize > Site Identity, or the theme's own.
			if ( has_custom_logo() ) {
				echo wp_get_attachment_image( (int) get_theme_mod( 'custom_logo' ), 'full', false, array( 'alt' => '', 'class' => 'brand-logo', 'loading' => false ) );
			} else {
				echo '<img class="brand-logo" src="' . esc_url( get_template_directory_uri() . '/assets/images/logo.svg' ) . '" alt="" width="52" height="40">';
			}
			?>
			<span><b><?php bloginfo( 'name' ); ?></b><small><?php bloginfo( 'description' ); ?></small></span>
		</a>
		<button class="menu-toggle" type="button" aria-expanded="false" aria-controls="primary-nav" aria-label="<?php esc_attr_e( 'Toggle navigation', 'voyagelight' ); ?>"><svg class="icon-open" viewBox="0 0 24 24" width="20" height="20" aria-hidden="true" focusable="false"><path d="M4 7h16M4 12h16M4 17h16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg><svg class="icon-close" viewBox="0 0 24 24" width="20" height="20" aria-hidden="true" focusable="false"><path d="M6 6l12 12M18 6L6 18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg></button>
		<nav id="primary-nav" class="primary-nav" aria-label="<?php esc_attr_e( 'Primary navigation', 'voyagelight' ); ?>">
			<?php
			if ( has_nav_menu( 'primary' ) ) {
				wp_nav_menu( array( 'theme_location' => 'primary', 'container' => false, 'menu_class' => 'nav-list' ) );
			} else {
				$stories = get_option( 'page_for_posts' ) ? get_permalink( get_option( 'page_for_posts' ) ) : home_url( '/?post_type=post' );
				echo '<ul class="nav-list">';
				echo '<li><a href="' . esc_url( home_url( '/' ) ) . '">' . esc_html__( 'Home', 'voyagelight' ) . '</a></li>';
				echo vl_category_menu_items(); // phpcs:ignore WordPress.Security.EscapeOutput.OutputNotEscaped -- escaped in vl_category_menu_items().
				echo '<li><a href="' . esc_url( $stories ) . '">' . esc_html__( 'Stories', 'voyagelight' ) . '</a></li>';
				echo '<li><a href="' . esc_url( get_post_type_archive_link( 'vl_trip' ) ) . '">' . esc_html__( 'Journeys', 'voyagelight' ) . '</a></li>';
				echo '</ul>';
			}
			?>
		</nav>
		<div class="header-actions">
			<?php if ( vl_color_toggle() ) : ?>
			<?php // A toggle named by its visible label: pressed while the page is dark. ?>
			<button class="theme-toggle" type="button" aria-pressed="false"><svg class="mode-icon" viewBox="0 0 24 24" width="16" height="16" aria-hidden="true" focusable="false"><path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/></svg><span class="mode-label"><?php esc_html_e( 'Dark', 'voyagelight' ); ?></span></button>
			<?php endif; ?>
		</div>
	</div>
</header>
<?php
// The phone menu whenever the header's links don't fit on one line, not only on a narrow screen,
// so a long menu never runs into the buttons. Runs here, before the page is drawn; theme.js isn't needed.
wp_print_inline_script_tag( "(function(){var h=document.querySelector('.site-header'),i=h&&h.firstElementChild;if(!i)return;function fit(){h.classList.remove('is-compact');h.classList.add('is-fitting');var c=window.innerWidth<=760||i.scrollWidth>i.clientWidth+1;h.classList.remove('is-fitting');h.classList.toggle('is-compact',c);}fit();window.addEventListener('resize',fit);if(document.fonts)document.fonts.ready.then(fit);window.vlFitHeader=fit;}());" );
