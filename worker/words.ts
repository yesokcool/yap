// Short, common, inoffensive words for temporary passwords (an owner or mod
// resets someone's password: "maple-river-stone-42"). Easy to read out loud
// and to type on a phone: lowercase, 3–6 letters, no look-alikes that need
// spelling out. Written for Yap (same licence as the rest of it).
// 3 words from these + 2 digits ≈ 3.5 billion choices; with the sign-in
// lockout (5 tries, then waits that double) nobody guesses one online.
export const WORDS = `
acorn amber apple apron arrow aspen atlas autumn badge bagel baker bamboo banjo barn basil basket beach beacon
bean bell berry birch bird bison bloom blue boat bonnet book branch brave bread breeze brick bridge bright
brook brush bubble bucket bunny butter button cabin cactus cake calm camel candle canoe canyon cape carrot
castle cedar chalk cherry chess chime cinema circle city clay cliff clock cloud clover coast cobalt cocoa
comet copper coral cotton cove cozy crane crayon creek crisp crown cup daisy dawn deer delta desert dew diary
dino donut dove dream drum dune eagle easel echo elbow elk elm ember fable falcon fern ferry fiddle field
finch flag flame flint flute foam forest fox frost fudge galaxy garden gecko gentle giant ginger glade glow
goose grape gravel green grove gull happy harbor harp hawk hazel heron hill honey hoop horse igloo indigo ink
iris island ivory ivy jacket jade jam jasper jelly jolly kayak kelp kettle kind kite kiwi koala ladder lagoon
lake lamb lamp lark lava leaf lemon lilac lily lime linen lion llama lobby lotus lucky lunar mango maple
marble meadow mellow melon merry mint mitten moon moose mossy muffin mural nest noodle north nutmeg oak oasis
ocean olive onion opal orange orbit orchid otter owl paddle panda paper parade parrot pasta peach pear pearl
pebble pecan pencil pepper petal piano pickle pine pixel planet plum pocket polar pond pony poppy prism puffin
quail quartz quiet quilt rabbit radar rain raisin raven reed reef ribbon ridge ripple river robin rocket rose
ruby saddle sage sail salad salmon sandal satin scarf seal seed shell shore silk silver sketch sky sled slope
snow sofa spice spoon spring sprout spruce squash star stone storm stream sugar summit sunny swan swift syrup
table taco teal teapot thyme tide tiger toast tomato topaz torch tower trail tulip tundra turnip twig valley
velvet violet violin waffle wagon walnut walrus wave willow window winter wren yarn yellow yogurt zebra zesty
zinnia
`.trim().split(/\s+/);
