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
			<img src="<?php echo esc_url( get_template_directory_uri() . '/assets/images/logo.svg' ); ?>" alt="" width="52" height="40">
			<span><b><?php bloginfo( 'name' ); ?></b><small><?php bloginfo( 'description' ); ?></small></span>
		</a>
		<button class="menu-toggle" type="button" aria-expanded="false" aria-controls="primary-nav" aria-label="<?php esc_attr_e( 'Toggle navigation', 'voyagelight' ); ?>">☰</button>
		<nav id="primary-nav" class="primary-nav" aria-label="<?php esc_attr_e( 'Primary navigation', 'voyagelight' ); ?>">
			<?php
			if ( has_nav_menu( 'primary' ) ) {
				wp_nav_menu( array( 'theme_location' => 'primary', 'container' => false, 'menu_class' => 'nav-list' ) );
			} else {
				$stories = get_option( 'page_for_posts' ) ? get_permalink( get_option( 'page_for_posts' ) ) : home_url( '/?post_type=post' );
				echo '<ul class="nav-list">';
				echo '<li><a href="' . esc_url( home_url( '/' ) ) . '">' . esc_html__( 'Home', 'voyagelight' ) . '</a></li>';
				echo '<li><a href="' . esc_url( $stories ) . '">' . esc_html__( 'Stories', 'voyagelight' ) . '</a></li>';
				echo '<li><a href="' . esc_url( get_post_type_archive_link( 'vl_trip' ) ) . '">' . esc_html__( 'Journeys', 'voyagelight' ) . '</a></li>';
				echo '</ul>';
			}
			?>
		</nav>
		<div class="header-actions">
			<button class="theme-toggle" type="button" aria-label="<?php esc_attr_e( 'Toggle light or dark theme', 'voyagelight' ); ?>" aria-pressed="false"><span class="mode-icon">☾</span><span class="mode-label"><?php esc_html_e( 'Dark', 'voyagelight' ); ?></span></button>
		</div>
	</div>
</header>
