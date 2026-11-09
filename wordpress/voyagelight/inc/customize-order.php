<?php
/**
 * The Customizer's list for putting the start page's sections in order.
 * Loaded from vl_customize(), once WordPress has its own Customizer classes.
 *
 * @package VoyageLight
 */

/** A list of the start page's sections, each with buttons that move it up or down; the setting holds their keys, comma-separated. */
class VL_Order_Control extends WP_Customize_Control {
	/** @var string */
	public $type = 'vl_order';

	/** The list, with the sections in their saved order. */
	public function render_content() {
		$names = vl_home_section_names();
		?>
		<span class="customize-control-title"><?php echo esc_html( $this->label ); ?></span>
		<?php if ( $this->description ) : ?>
			<span class="description customize-control-description"><?php echo esc_html( $this->description ); ?></span>
		<?php endif; ?>
		<ol class="vl-order" data-setting="<?php echo esc_attr( $this->settings['default']->id ); ?>">
			<?php foreach ( vl_home_order( $this->value() ) as $key ) : ?>
				<li data-key="<?php echo esc_attr( $key ); ?>">
					<span class="vl-order-name"><?php echo esc_html( $names[ $key ] ); ?></span>
					<?php /* translators: %s: name of a start page section, such as Travel map. */ ?>
					<button type="button" class="button vl-order-up" aria-label="<?php echo esc_attr( sprintf( __( 'Move %s up', 'voyagelight' ), $names[ $key ] ) ); ?>">↑</button>
					<?php /* translators: %s: name of a start page section, such as Travel map. */ ?>
					<button type="button" class="button vl-order-down" aria-label="<?php echo esc_attr( sprintf( __( 'Move %s down', 'voyagelight' ), $names[ $key ] ) ); ?>">↓</button>
				</li>
			<?php endforeach; ?>
		</ol>
		<?php
	}
}
