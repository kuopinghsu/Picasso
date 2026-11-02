<?php $field = wp_unique_id( 'site-search-' ); ?>
<form role="search" method="get" class="search-form" action="<?php echo esc_url( home_url( '/' ) ); ?>">
	<label for="<?php echo esc_attr( $field ); ?>"><?php esc_html_e( 'Search the journal', 'voyagelight' ); ?></label>
	<div>
		<input id="<?php echo esc_attr( $field ); ?>" type="search" name="s" value="<?php echo esc_attr( get_search_query() ); ?>" placeholder="<?php esc_attr_e( 'Search stories, places…', 'voyagelight' ); ?>">
		<button type="submit"><?php esc_html_e( 'Search', 'voyagelight' ); ?></button>
	</div>
</form>
